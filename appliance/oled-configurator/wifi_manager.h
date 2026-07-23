#ifndef WIFI_MANAGER_H
#define WIFI_MANAGER_H

#include <stdbool.h>
#include <stddef.h>

#define WIFI_MAX_NETWORKS 64
#define WIFI_MAX_PROFILES 64
#define WIFI_NAME_MAX 128

typedef struct {
    char interface[32];
    char state[32];
    char ssid[WIFI_NAME_MAX];
    char connection[WIFI_NAME_MAX];
    char ip[48];
    char mode[32];
    int signal;
} wifi_status_t;

typedef struct {
    char ssid[WIFI_NAME_MAX];
    char security[64];
    int signal;
    bool active;
} wifi_network_t;

typedef struct {
    char hostname[64];
    char temp[16];
    char ram[48];
    char ip_wlan[48];
} sys_info_t;

typedef struct {
    char ssid[WIFI_NAME_MAX];
    bool exists;
} wifi_ap_config_t;

typedef enum {
    WIFI_BOOT_AP,
    WIFI_BOOT_AUTO_CLIENT,
    WIFI_BOOT_CLIENT_PROFILE
} wifi_boot_target_t;

int wifi_manager_init(const char *iface);
void wifi_manager_close(void);
void wifi_get_status(wifi_status_t *out_status);
void sys_get_info(sys_info_t *out_info);

size_t wifi_scan(wifi_network_t *networks, size_t capacity, char *error, size_t error_size);
size_t wifi_get_saved_profiles(char profiles[][WIFI_NAME_MAX], size_t capacity);
int wifi_connect(const char *ssid, const char *password, char *message, size_t message_size);
int wifi_connect_saved(const char *profile, char *message, size_t message_size);
int wifi_enable_ap(char *message, size_t message_size);
int wifi_disable_ap(char *message, size_t message_size);
int wifi_get_ap_config(wifi_ap_config_t *config);
int wifi_set_ap_config(const char *ssid, const char *password,
                       char *message, size_t message_size);
int wifi_set_boot_default(wifi_boot_target_t target, const char *profile,
                          char *message, size_t message_size);

#endif
