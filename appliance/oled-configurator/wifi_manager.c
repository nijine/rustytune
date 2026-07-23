#define _POSIX_C_SOURCE 200809L
#include "wifi_manager.h"

#include <arpa/inet.h>
#include <errno.h>
#include <ifaddrs.h>
#include <pthread.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static wifi_status_t cached_status;
static pthread_mutex_t status_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_t worker;
static bool worker_started;
static volatile sig_atomic_t running;
static char iface[32] = "wlan0";
static char hostname_cache[64] = "raspberrypi";
static const char ap_profile_name[] = "Pi-Configurator-AP";
static const char ap_address[] = "10.0.0.1/24";

static int find_config_ap(char *uuid, size_t uuid_size, char *ssid, size_t ssid_size);
static int apply_ap_network_policy(const char *uuid, char *output, size_t output_size);

static void set_message(char *dst, size_t size, const char *fmt, ...) {
    if (!dst || size == 0) return;
    va_list args;
    va_start(args, fmt);
    vsnprintf(dst, size, fmt, args);
    va_end(args);
}

/* Execute without a shell: SSIDs and passwords can never become shell syntax. */
static int run_capture(const char *const argv[], char *output, size_t output_size) {
    int pipefd[2];
    if (output && output_size) output[0] = '\0';
    if (pipe(pipefd) != 0) return -1;

    pid_t pid = fork();
    if (pid == 0) {
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[0]);
        close(pipefd[1]);
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    close(pipefd[1]);
    if (pid < 0) {
        close(pipefd[0]);
        return -1;
    }

    size_t used = 0;
    char discard[256];
    for (;;) {
        char *target = discard;
        size_t room = sizeof(discard);
        if (output && output_size && used + 1 < output_size) {
            target = output + used;
            room = output_size - used - 1;
        }
        ssize_t n = read(pipefd[0], target, room);
        if (n <= 0) break;
        if (target != discard) used += (size_t)n;
    }
    close(pipefd[0]);
    if (output && output_size) {
        output[used] = '\0';
        while (used && (output[used - 1] == '\n' || output[used - 1] == '\r')) output[--used] = '\0';
    }
    int status = 0;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {}
    return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}

/* Split nmcli's terse output while honoring its backslash-escaped colons. */
static size_t split_terse(char *line, char **fields, size_t capacity) {
    size_t count = 0;
    char *src = line, *dst = line;
    if (capacity) fields[count++] = dst;
    while (*src) {
        if (*src == '\\' && src[1]) {
            *dst++ = src[1];
            src += 2;
        } else if (*src == ':' && count < capacity) {
            *dst++ = '\0';
            src++;
            fields[count++] = dst;
        } else {
            *dst++ = *src++;
        }
    }
    *dst = '\0';
    return count;
}

static void lookup_ip(char *dst, size_t size) {
    snprintf(dst, size, "No IP");
    struct ifaddrs *addresses = NULL;
    if (getifaddrs(&addresses) != 0) return;
    for (struct ifaddrs *it = addresses; it; it = it->ifa_next) {
        if (!it->ifa_addr || strcmp(it->ifa_name, iface) != 0 || it->ifa_addr->sa_family != AF_INET) continue;
        inet_ntop(AF_INET, &((struct sockaddr_in *)it->ifa_addr)->sin_addr, dst, (socklen_t)size);
        break;
    }
    freeifaddrs(addresses);
}

static void update_status(void) {
    wifi_status_t next = {0};
    snprintf(next.interface, sizeof(next.interface), "%s", iface);
    snprintf(next.state, sizeof(next.state), "Disconnected");
    snprintf(next.ssid, sizeof(next.ssid), "Disconnected");
    snprintf(next.connection, sizeof(next.connection), "Disconnected");
    snprintf(next.mode, sizeof(next.mode), "Client");
    lookup_ip(next.ip, sizeof(next.ip));

    char output[8192];
    const char *dev_args[] = {"nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev", NULL};
    if (run_capture(dev_args, output, sizeof(output)) == 0) {
        char *save = NULL;
        for (char *line = strtok_r(output, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
            char *field[4];
            if (split_terse(line, field, 4) >= 4 && strcmp(field[0], iface) == 0) {
                snprintf(next.state, sizeof(next.state), "%s", field[2]);
                if (field[3][0] && strcmp(field[3], "--") != 0)
                    snprintf(next.connection, sizeof(next.connection), "%s", field[3]);
                break;
            }
        }
    }

    if (strcmp(next.connection, "Disconnected") != 0) {
        snprintf(next.ssid, sizeof(next.ssid), "%s", next.connection);
        const char *mode_args[] = {"nmcli", "-g", "802-11-wireless.mode", "con", "show", next.connection, NULL};
        if (run_capture(mode_args, output, sizeof(output)) == 0 && strcmp(output, "ap") == 0)
            snprintf(next.mode, sizeof(next.mode), "AP Hotspot");
        const char *ssid_args[] = {"nmcli", "-g", "802-11-wireless.ssid", "con", "show", next.connection, NULL};
        if (run_capture(ssid_args, output, sizeof(output)) == 0 && output[0])
            snprintf(next.ssid, sizeof(next.ssid), "%.127s", output);
    }
    if (strcmp(next.mode, "Client") == 0) {
        const char *signal_args[] = {"nmcli", "-t", "-f", "IN-USE,SIGNAL", "dev", "wifi", "list", "ifname", iface, NULL};
        if (run_capture(signal_args, output, sizeof(output)) == 0) {
            char *save = NULL;
            for (char *line = strtok_r(output, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
                char *field[2];
                if (split_terse(line, field, 2) == 2 && strcmp(field[0], "*") == 0) { next.signal = atoi(field[1]); break; }
            }
        }
    }

    pthread_mutex_lock(&status_mutex);
    cached_status = next;
    pthread_mutex_unlock(&status_mutex);
}

static void *poll_worker(void *unused) {
    (void)unused;
    while (running) {
        update_status();
        for (int i = 0; i < 30 && running; ++i) {
            struct timespec delay = {.tv_sec = 0, .tv_nsec = 100000000L};
            nanosleep(&delay, NULL);
        }
    }
    return NULL;
}

int wifi_manager_init(const char *requested_iface) {
    if (requested_iface && *requested_iface) snprintf(iface, sizeof(iface), "%s", requested_iface);
    gethostname(hostname_cache, sizeof(hostname_cache) - 1);
    char uuid[40], ssid[WIFI_NAME_MAX], output[2048];
    if (find_config_ap(uuid, sizeof(uuid), ssid, sizeof(ssid)) == 0)
        (void)apply_ap_network_policy(uuid, output, sizeof(output));
    update_status();
    running = 1;
    if (pthread_create(&worker, NULL, poll_worker, NULL) != 0) return -1;
    worker_started = true;
    return 0;
}

void wifi_manager_close(void) {
    running = 0;
    if (worker_started) pthread_join(worker, NULL);
    worker_started = false;
}

void wifi_get_status(wifi_status_t *out_status) {
    pthread_mutex_lock(&status_mutex);
    *out_status = cached_status;
    pthread_mutex_unlock(&status_mutex);
}

static int network_compare(const void *a, const void *b) {
    return ((const wifi_network_t *)b)->signal - ((const wifi_network_t *)a)->signal;
}

typedef struct {
    char uuid[40];
    unsigned long long timestamp;
} client_candidate_t;

static int client_candidate_compare(const void *a, const void *b) {
    const client_candidate_t *left = a;
    const client_candidate_t *right = b;
    if (left->timestamp < right->timestamp) return 1;
    if (left->timestamp > right->timestamp) return -1;
    return 0;
}

size_t wifi_scan(wifi_network_t *networks, size_t capacity, char *error, size_t error_size) {
    char output[32768];
    const char *radio[] = {"sudo", "-n", "nmcli", "radio", "wifi", "on", NULL};
    (void)run_capture(radio, output, sizeof(output));
    const char *rescan[] = {"sudo", "-n", "nmcli", "dev", "wifi", "rescan", "ifname", iface, NULL};
    (void)run_capture(rescan, output, sizeof(output));
    const char *args[] = {"nmcli", "-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE", "dev", "wifi", "list", "ifname", iface, NULL};
    int code = run_capture(args, output, sizeof(output));
    if (code != 0) { set_message(error, error_size, "%s", output[0] ? output : "Wi-Fi scan failed"); return 0; }

    size_t count = 0;
    char *save = NULL;
    for (char *line = strtok_r(output, "\n", &save); line && count < capacity; line = strtok_r(NULL, "\n", &save)) {
        char *field[4];
        if (split_terse(line, field, 4) < 4 || !field[0][0]) continue;
        bool duplicate = false;
        for (size_t i = 0; i < count; ++i) if (strcmp(networks[i].ssid, field[0]) == 0) { duplicate = true; break; }
        if (duplicate) continue;
        snprintf(networks[count].ssid, sizeof(networks[count].ssid), "%s", field[0]);
        snprintf(networks[count].security, sizeof(networks[count].security), "%s", field[2][0] ? field[2] : "Open");
        networks[count].signal = atoi(field[1]);
        networks[count].active = strcmp(field[3], "*") == 0;
        count++;
    }
    qsort(networks, count, sizeof(*networks), network_compare);
    return count;
}

size_t wifi_get_saved_profiles(char profiles[][WIFI_NAME_MAX], size_t capacity) {
    char output[16384];
    const char *args[] = {"nmcli", "-t", "-f", "NAME,TYPE", "con", "show", NULL};
    if (run_capture(args, output, sizeof(output)) != 0) return 0;
    size_t count = 0;
    char *save = NULL;
    for (char *line = strtok_r(output, "\n", &save); line && count < capacity; line = strtok_r(NULL, "\n", &save)) {
        char *field[2];
        if (split_terse(line, field, 2) == 2 && strcmp(field[1], "802-11-wireless") == 0) {
            char mode[32];
            const char *mode_args[] = {"nmcli", "-g", "802-11-wireless.mode", "con", "show", field[0], NULL};
            if (run_capture(mode_args, mode, sizeof(mode)) == 0 && strcmp(mode, "ap") != 0)
                snprintf(profiles[count++], WIFI_NAME_MAX, "%s", field[0]);
        }
    }
    return count;
}

int wifi_connect(const char *ssid, const char *password, char *message, size_t message_size) {
    char output[2048];
    const char *open_args[] = {"sudo", "-n", "nmcli", "dev", "wifi", "connect", ssid, "ifname", iface, NULL};
    const char *secure_args[] = {"sudo", "-n", "nmcli", "dev", "wifi", "connect", ssid, "password", password, "ifname", iface, NULL};
    int code = run_capture(password && *password ? secure_args : open_args, output, sizeof(output));
    update_status();
    set_message(message, message_size, "%s", code == 0 ? "Connected" : (output[0] ? output : "Connection failed"));
    return code == 0 ? 0 : -1;
}

int wifi_connect_saved(const char *profile, char *message, size_t message_size) {
    char output[2048];
    const char *args[] = {"sudo", "-n", "nmcli", "con", "up", "id", profile, NULL};
    int code = run_capture(args, output, sizeof(output));
    update_status();
    set_message(message, message_size, "%s", code == 0 ? "Profile activated" : (output[0] ? output : "Activation failed"));
    return code == 0 ? 0 : -1;
}

static int find_config_ap(char *uuid, size_t uuid_size, char *ssid, size_t ssid_size) {
    char output[16384];
    const char *args[] = {"nmcli", "-t", "-f", "UUID,NAME,TYPE", "con", "show", NULL};
    if (run_capture(args, output, sizeof(output)) != 0) return -1;
    char fallback_uuid[40] = "", fallback_ssid[WIFI_NAME_MAX] = "";
    char *save = NULL;
    for (char *line = strtok_r(output, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
        char *field[3];
        if (split_terse(line, field, 3) < 3 || strcmp(field[2], "802-11-wireless") != 0) continue;
        char mode[32], profile_ssid[WIFI_NAME_MAX];
        const char *mode_args[] = {"nmcli", "-g", "802-11-wireless.mode", "con", "show", "uuid", field[0], NULL};
        const char *ssid_args[] = {"nmcli", "-g", "802-11-wireless.ssid", "con", "show", "uuid", field[0], NULL};
        if (run_capture(mode_args, mode, sizeof(mode)) != 0 || strcmp(mode, "ap") != 0) continue;
        (void)run_capture(ssid_args, profile_ssid, sizeof(profile_ssid));
        if (strcmp(field[1], ap_profile_name) == 0) {
            snprintf(uuid, uuid_size, "%s", field[0]);
            snprintf(ssid, ssid_size, "%s", profile_ssid);
            return 0;
        }
        if (!fallback_uuid[0] && strcmp(profile_ssid, "Pi-AP") == 0) {
            snprintf(fallback_uuid, sizeof(fallback_uuid), "%s", field[0]);
            snprintf(fallback_ssid, sizeof(fallback_ssid), "%s", profile_ssid);
        }
    }
    if (fallback_uuid[0]) {
        snprintf(uuid, uuid_size, "%s", fallback_uuid);
        snprintf(ssid, ssid_size, "%s", fallback_ssid);
        return 0;
    }
    return -1;
}

static int apply_ap_network_policy(const char *uuid, char *output, size_t output_size) {
    const char *args[] = {
        "sudo", "-n", "nmcli", "con", "modify", "uuid", uuid,
        "ipv4.method", "shared",
        "ipv4.addresses", ap_address,
        "ipv4.never-default", "yes",
        "ipv6.method", "disabled",
        NULL
    };
    return run_capture(args, output, output_size);
}

int wifi_get_ap_config(wifi_ap_config_t *config) {
    memset(config, 0, sizeof(*config));
    char uuid[40];
    config->exists = find_config_ap(uuid, sizeof(uuid), config->ssid, sizeof(config->ssid)) == 0;
    if (!config->exists) snprintf(config->ssid, sizeof(config->ssid), "Pi-AP");
    return 0;
}

int wifi_set_ap_config(const char *ssid, const char *password,
                       char *message, size_t message_size) {
    if (!ssid || !ssid[0] || strlen(ssid) > 32) {
        set_message(message, message_size, "SSID must be 1-32 chars");
        return -1;
    }
    if (password && password[0] && (strlen(password) < 8 || strlen(password) > 63)) {
        set_message(message, message_size, "Password must be 8-63");
        return -1;
    }

    char uuid[40], old_ssid[WIFI_NAME_MAX], output[4096];
    bool exists = find_config_ap(uuid, sizeof(uuid), old_ssid, sizeof(old_ssid)) == 0;
    if (!exists) {
        if (!password || !password[0]) {
            set_message(message, message_size, "Set an AP password first");
            return -1;
        }
        const char *add[] = {"sudo", "-n", "nmcli", "con", "add", "type", "wifi", "ifname", iface,
                             "con-name", ap_profile_name, "autoconnect", "no", "ssid", ssid, NULL};
        if (run_capture(add, output, sizeof(output)) != 0 ||
            find_config_ap(uuid, sizeof(uuid), old_ssid, sizeof(old_ssid)) != 0) {
            set_message(message, message_size, "%s", output[0] ? output : "Could not create AP profile");
            return -1;
        }
    }

    const char *base[] = {"sudo", "-n", "nmcli", "con", "modify", "uuid", uuid,
                          "connection.id", ap_profile_name, "802-11-wireless.mode", "ap",
                          "802-11-wireless.ssid", ssid,
                          "802-11-wireless-security.key-mgmt", "wpa-psk", NULL};
    if (run_capture(base, output, sizeof(output)) != 0) {
        set_message(message, message_size, "%s", output[0] ? output : "Could not update AP profile");
        return -1;
    }
    if (apply_ap_network_policy(uuid, output, sizeof(output)) != 0) {
        set_message(message, message_size, "%s", output[0] ? output : "Could not apply AP network policy");
        return -1;
    }
    if (password && password[0]) {
        const char *secret[] = {"sudo", "-n", "nmcli", "con", "modify", "uuid", uuid,
                                "802-11-wireless-security.psk", password, NULL};
        if (run_capture(secret, output, sizeof(output)) != 0) {
            set_message(message, message_size, "%s", output[0] ? output : "Could not set AP password");
            return -1;
        }
    }
    set_message(message, message_size, "Saved AP: %s", ssid);
    return 0;
}

int wifi_enable_ap(char *message, size_t message_size) {
    wifi_ap_config_t config;
    wifi_get_ap_config(&config);
    if (!config.exists) {
        if (wifi_set_ap_config("Pi-AP", "raspberry", message, message_size) != 0) return -1;
    }
    char uuid[40], ssid[WIFI_NAME_MAX], output[2048];
    if (find_config_ap(uuid, sizeof(uuid), ssid, sizeof(ssid)) != 0) {
        set_message(message, message_size, "AP profile not found");
        return -1;
    }
    const char *args[] = {"sudo", "-n", "nmcli", "con", "up", "uuid", uuid, "ifname", iface, NULL};
    int code = run_capture(args, output, sizeof(output));
    update_status();
    set_message(message, message_size, "%s", code == 0 ? "AP hotspot active" : (output[0] ? output : "AP activation failed"));
    return code == 0 ? 0 : -1;
}

int wifi_disable_ap(char *message, size_t message_size) {
    wifi_status_t status;
    wifi_get_status(&status);
    if (strcmp(status.mode, "AP Hotspot") != 0 || !status.connection[0] ||
        !strcmp(status.connection, "Disconnected")) {
        set_message(message, message_size, "AP mode is not active");
        return -1;
    }

    char output[16384];
    client_candidate_t candidates[WIFI_MAX_PROFILES];
    size_t candidate_count = 0;
    const char *list_args[] = {"nmcli", "-t", "-f", "UUID,TYPE,TIMESTAMP", "con", "show", NULL};
    if (run_capture(list_args, output, sizeof(output)) == 0) {
        char *save = NULL;
        for (char *line = strtok_r(output, "\n", &save); line && candidate_count < WIFI_MAX_PROFILES;
             line = strtok_r(NULL, "\n", &save)) {
            char *field[3];
            if (split_terse(line, field, 3) < 3 || strcmp(field[1], "802-11-wireless") != 0) continue;
            char mode[32];
            const char *mode_args[] = {"nmcli", "-g", "802-11-wireless.mode", "con", "show", "uuid", field[0], NULL};
            if (run_capture(mode_args, mode, sizeof(mode)) != 0 || strcmp(mode, "ap") == 0) continue;
            snprintf(candidates[candidate_count].uuid, sizeof(candidates[candidate_count].uuid), "%s", field[0]);
            candidates[candidate_count].timestamp = strtoull(field[2], NULL, 10);
            candidate_count++;
        }
    }
    if (candidate_count == 0) {
        set_message(message, message_size, "No saved client networks");
        return -1;
    }
    qsort(candidates, candidate_count, sizeof(*candidates), client_candidate_compare);

    for (size_t i = 0; i < candidate_count; ++i) {
        const char *connect[] = {
            "sudo", "-n", "nmcli", "con", "up", "uuid", candidates[i].uuid, "ifname", iface, NULL
        };
        if (run_capture(connect, output, sizeof(output)) == 0) {
            update_status();
            wifi_get_status(&status);
            set_message(message, message_size, "Connected to %s", status.ssid);
            return 0;
        }
    }
    update_status();
    set_message(message, message_size, "%s", output[0] ? output : "Choose a saved network");
    return -1;
}

typedef struct {
    char uuid[40];
    char name[WIFI_NAME_MAX];
    char mode[32];
    char ssid[WIFI_NAME_MAX];
} boot_profile_t;

int wifi_set_boot_default(wifi_boot_target_t target, const char *profile,
                          char *message, size_t message_size) {
    char output[16384];
    boot_profile_t entries[WIFI_MAX_PROFILES];
    size_t count = 0;
    const char *list_args[] = {"nmcli", "-t", "-f", "UUID,NAME,TYPE", "con", "show", NULL};
    if (run_capture(list_args, output, sizeof(output)) != 0) {
        set_message(message, message_size, "%s", output[0] ? output : "Could not list profiles");
        return -1;
    }

    char *save = NULL;
    for (char *line = strtok_r(output, "\n", &save); line && count < WIFI_MAX_PROFILES;
         line = strtok_r(NULL, "\n", &save)) {
        char *field[3];
        if (split_terse(line, field, 3) < 3 || strcmp(field[2], "802-11-wireless") != 0) continue;
        snprintf(entries[count].uuid, sizeof(entries[count].uuid), "%s", field[0]);
        snprintf(entries[count].name, sizeof(entries[count].name), "%s", field[1]);
        const char *mode_args[] = {
            "nmcli", "-g", "802-11-wireless.mode", "con", "show", "uuid", entries[count].uuid, NULL
        };
        if (run_capture(mode_args, entries[count].mode, sizeof(entries[count].mode)) != 0) continue;
        const char *ssid_args[] = {
            "nmcli", "-g", "802-11-wireless.ssid", "con", "show", "uuid", entries[count].uuid, NULL
        };
        (void)run_capture(ssid_args, entries[count].ssid, sizeof(entries[count].ssid));
        count++;
    }

    ssize_t selected = -1;
    for (size_t i = 0; i < count; ++i) {
        bool is_ap = strcmp(entries[i].mode, "ap") == 0;
        if (target == WIFI_BOOT_AP && is_ap &&
            (strcmp(entries[i].name, ap_profile_name) == 0 || strcmp(entries[i].ssid, "Pi-AP") == 0))
            selected = (ssize_t)i;
        if (target == WIFI_BOOT_CLIENT_PROFILE && !is_ap && profile && strcmp(entries[i].name, profile) == 0)
            selected = (ssize_t)i;
    }
    if (target != WIFI_BOOT_AUTO_CLIENT && selected < 0) {
        set_message(message, message_size, "%s",
                    target == WIFI_BOOT_AP ? "Start AP mode once first" : "Client profile not found");
        return -1;
    }

    for (size_t i = 0; i < count; ++i) {
        bool is_ap = strcmp(entries[i].mode, "ap") == 0;
        const char *priority_args[] = {
            "sudo", "-n", "nmcli", "con", "modify", "uuid", entries[i].uuid,
            "connection.autoconnect-priority", i == (size_t)selected ? "999" : "0", NULL
        };
        if (run_capture(priority_args, output, sizeof(output)) != 0) {
            set_message(message, message_size, "%s", output[0] ? output : "Could not set priority");
            return -1;
        }

        const char *autoconnect = "yes";
        if (is_ap && !(target == WIFI_BOOT_AP && i == (size_t)selected)) autoconnect = "no";
        const char *auto_args[] = {
            "sudo", "-n", "nmcli", "con", "modify", "uuid", entries[i].uuid,
            "connection.autoconnect", autoconnect, NULL
        };
        if (run_capture(auto_args, output, sizeof(output)) != 0) {
            set_message(message, message_size, "%s", output[0] ? output : "Could not set autoconnect");
            return -1;
        }
    }

    if (target == WIFI_BOOT_AP) set_message(message, message_size, "Default: AP Pi-AP");
    else if (target == WIFI_BOOT_AUTO_CLIENT) set_message(message, message_size, "Default: Auto client");
    else set_message(message, message_size, "Default: %s", profile);
    return 0;
}

void sys_get_info(sys_info_t *out) {
    memset(out, 0, sizeof(*out));
    snprintf(out->hostname, sizeof(out->hostname), "%s", hostname_cache);
    snprintf(out->temp, sizeof(out->temp), "N/A");
    snprintf(out->ram, sizeof(out->ram), "N/A");
    FILE *fp = fopen("/sys/class/thermal/thermal_zone0/temp", "r");
    int milli;
    if (fp && fscanf(fp, "%d", &milli) == 1) snprintf(out->temp, sizeof(out->temp), "%.1f C", milli / 1000.0);
    if (fp) fclose(fp);
    fp = fopen("/proc/meminfo", "r");
    long total = 0, available = 0;
    if (fp) {
        char line[128];
        while (fgets(line, sizeof(line), fp)) {
            (void)sscanf(line, "MemTotal: %ld kB", &total);
            (void)sscanf(line, "MemAvailable: %ld kB", &available);
        }
        fclose(fp);
    }
    if (total > 0) snprintf(out->ram, sizeof(out->ram), "%ld/%ldMB", (total - available) / 1024, total / 1024);
    wifi_status_t status;
    wifi_get_status(&status);
    snprintf(out->ip_wlan, sizeof(out->ip_wlan), "%s", status.ip);
}
