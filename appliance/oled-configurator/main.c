#define _DEFAULT_SOURCE
#include <ctype.h>
#include <errno.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <dirent.h>
#include <sys/statvfs.h>

#include "gpio_buttons.h"
#include "ssd1306.h"
#include "system_control.h"
#include "wifi_manager.h"
#include "rustytune_client.h"

typedef enum { MAIN_MENU, WIFI_MENU, SYSTEM_MENU, RUSTYTUNE_MENU, RUSTYTUNE_STATUS, RUSTYTUNE_CONNECTION, RUSTYTUNE_PAIR, RUSTYTUNE_STORAGE, WIFI_STATUS, WIFI_SCAN, WIFI_SAVED, OSK_PASS, AP_SETTINGS, BOOT_NETWORK, CONFIRM_AP_RESET, CONFIRM_AP_RESTART, CONFIRM_CLIENT, CONFIRM_REBOOT, CONFIRM_SHUTDOWN, DIALOG, SYS_INFO } app_state_t;
typedef enum { INPUT_WIFI_PASSWORD, INPUT_AP_SSID, INPUT_AP_PASSWORD } input_mode_t;
typedef struct { bool down; double pressed_at; double repeated_at; } button_state_t;

static volatile sig_atomic_t running = 1;
static void handle_signal(int sig) { (void)sig; running = 0; }
static double now_seconds(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void draw_header(ssd1306_t *d, const char *title, const char *badge) {
    ssd1306_draw_rect(d, 0, 0, 128, 13, 1, true);
    ssd1306_draw_string(d, 3, 2, title, 0, 1);
    if (badge) {
        int x = 124 - (int)strlen(badge) * 6;
        ssd1306_draw_rect(d, x, 1, 127 - x, 11, 0, true);
        ssd1306_draw_string(d, x + 2, 2, badge, 1, 0);
    }
    ssd1306_draw_line(d, 0, 13, 127, 13, 1);
}

static void ellipsize(char *dst, size_t size, const char *src, size_t chars) {
    if (!size) return;
    if (!src) src = "";
    size_t length = strlen(src);
    size_t limit = chars < size - 1 ? chars : size - 1;
    size_t copied = length < limit ? length : limit;
    memcpy(dst, src, copied);
    dst[copied] = '\0';
    if (length > limit && copied >= 2) {
        dst[copied - 2] = '.';
        dst[copied - 1] = '.';
    }
}

static void render_menu(ssd1306_t *d, const char *title, char items[][32], size_t count, size_t selected, const char *badge) {
    ssd1306_clear(d); draw_header(d, title, badge);
    size_t top = selected >= 4 ? selected - 3 : 0;
    for (size_t row = 0; row < 4 && top + row < count; ++row) {
        size_t idx = top + row; int y = 15 + (int)row * 12; char text[21];
        snprintf(text, sizeof(text), "%c %.17s", idx == selected ? '>' : ' ', items[idx]);
        if (idx == selected) ssd1306_draw_rect(d, 2, y, 120, 11, 1, true);
        ssd1306_draw_string(d, 4, y + 2, text, idx == selected ? 0 : 1, idx == selected ? 1 : 0);
    }
    if (count > 1) {
        ssd1306_draw_line(d, 125, 15, 125, 62, 1);
        int y = 15 + (int)(selected * 41 / (count - 1));
        ssd1306_draw_rect(d, 124, y, 3, 6, 1, true);
    }
}

static void render_card(ssd1306_t *d, const char *title, char lines[][22], size_t count) {
    ssd1306_clear(d); draw_header(d, title, NULL);
    for (size_t i = 0; i < count && i < 4; ++i) ssd1306_draw_string(d, 4, 16 + (int)i * 10, lines[i], 1, 0);
    ssd1306_draw_line(d, 0, 53, 127, 53, 1); ssd1306_draw_string(d, 4, 55, "[#6: Back]", 1, 0);
}

static void render_info_list(ssd1306_t *d, const char *title, char lines[][32],
                             size_t count, size_t top) {
    ssd1306_clear(d); draw_header(d, title, NULL);
    for (size_t row = 0; row < 4 && top + row < count; ++row) {
        char visible[22];
        ellipsize(visible, sizeof(visible), lines[top + row], 20);
        ssd1306_draw_string(d, 4, 16 + (int)row * 10, visible, 1, 0);
    }
    if (count > 4) {
        ssd1306_draw_line(d, 125, 15, 125, 54, 1);
        int thumb_height = 39 * 4 / (int)count;
        if (thumb_height < 6) thumb_height = 6;
        int travel = 39 - thumb_height;
        int y = 15 + (count > 4 ? (int)(top * (size_t)travel / (count - 4)) : 0);
        ssd1306_draw_rect(d, 124, y, 3, thumb_height, 1, true);
    }
    ssd1306_draw_line(d, 0, 55, 127, 55, 1);
    ssd1306_draw_string(d, 4, 57, "Up/Down  #6:Back", 1, 0);
}

static void render_dialog(ssd1306_t *d, const char *title, const char *message, const char *footer) {
    char line1[21] = {0}, line2[21] = {0};
    snprintf(line1, sizeof(line1), "%.20s", message);
    if (strlen(message) > 20) snprintf(line2, sizeof(line2), "%.20s", message + 20);
    ssd1306_clear(d); ssd1306_draw_rect(d, 4, 4, 120, 56, 1, false);
    ssd1306_draw_rect(d, 4, 4, 120, 13, 1, true); ssd1306_draw_string(d, 8, 6, title, 0, 1);
    ssd1306_draw_string(d, 8, 22, line1, 1, 0); ssd1306_draw_string(d, 8, 32, line2, 1, 0);
    ssd1306_draw_line(d, 4, 48, 123, 48, 1); ssd1306_draw_string(d, 8, 51, footer, 1, 0);
}

static const char *keyboard[6][8] = {
    {"a","b","c","d","e","f","g","h"}, {"i","j","k","l","m","n","o","p"},
    {"q","r","s","t","u","v","w","x"}, {"y","z","0","1","2","3","4","5"},
    {"6","7","8","9","!","@","#","$"}, {"-","_","."," ","<","^","OK",NULL}
};

static void render_keyboard(ssd1306_t *d, const char *title, const char *text, bool mask,
                            bool uppercase, int col, int row) {
    char hidden[128], header[22];
    size_t len = strlen(text);
    if (mask) { size_t n = len < sizeof(hidden) - 1 ? len : sizeof(hidden) - 1; memset(hidden, '*', n); hidden[n] = '\0'; }
    const char *shown = mask ? hidden : text;
    size_t shown_len = strlen(shown);
    const char *tail = shown_len > 12 ? shown + shown_len - 12 : shown;
    snprintf(header, sizeof(header), "%.4s:%.12s%c", title, tail, uppercase ? '^' : '_');
    ssd1306_clear(d); ssd1306_draw_rect(d, 0, 0, 128, 14, 1, true); ssd1306_draw_string(d, 2, 2, header, 0, 1);
    for (int r = 0; r < 6; ++r) for (int c = 0; c < 8 && keyboard[r][c]; ++c) {
        int x = 2 + c * 15, y = 16 + r * 8; bool selected = r == row && c == col;
        int width = strlen(keyboard[r][c]) > 1 ? 22 : 13;
        if (selected) ssd1306_draw_rect(d, x, y, width, 8, 1, true);
        ssd1306_draw_string(d, x + 1, y, keyboard[r][c], selected ? 0 : 1, selected ? 1 : 0);
    }
}

static int triggered_button(button_state_t states[BTN_COUNT], double now, bool *activity) {
    for (int i = 0; i < BTN_COUNT; ++i) {
        bool pressed = gpio_button_is_pressed((button_id_t)i);
        if (pressed && !states[i].down) {
            states[i] = (button_state_t){true, now, now}; *activity = true; return i;
        }
        if (pressed && now - states[i].pressed_at >= .22 && now - states[i].repeated_at >= .08) {
            states[i].repeated_at = now; *activity = true; return i;
        }
        if (!pressed) states[i] = (button_state_t){0};
    }
    return -1;
}

static void usage(const char *program) {
    printf("Usage: %s [--address 0x3c] [--dev /dev/i2c-1] [--interface wlan0] [--idle-dim SEC] [--idle-blank SEC]\n", program);
}

static void detect_serial(char *out, size_t size) {
    const char *preferred[] = {"/dev/serial0", "/dev/ttyACM0", "/dev/ttyUSB0", "/dev/ttyAMA0"};
    for (size_t i=0;i<sizeof(preferred)/sizeof(preferred[0]);++i) if(access(preferred[i],F_OK)==0){snprintf(out,size,"%s",preferred[i]);return;}
    DIR *dir=opendir("/dev"); if(!dir)return; struct dirent *entry;
    while ((entry = readdir(dir))) {
        if (!strncmp(entry->d_name, "ttyACM", 6) || !strncmp(entry->d_name, "ttyUSB", 6)) {
            snprintf(out, size, "/dev/%.80s", entry->d_name);
            break;
        }
    }
    closedir(dir);
}

static const char *rustytune_ecu_state(const rustytune_status_t *status) {
    if (!status->connected) return "disconnected";
    if (strstr(status->error, "ECU not responding")) {
        return status->frames ? "signal lost" : "not responding";
    }
    if (!status->frames) return "waiting";
    return "connected";
}

int main(int argc, char **argv) {
    const char *i2c_device = "/dev/i2c-1", *interface = "wlan0";
    unsigned long address = 0x3c; int idle_dim = 30, idle_blank = 60;
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--help") || !strcmp(argv[i], "-h")) { usage(argv[0]); return 0; }
        else if (!strcmp(argv[i], "--address") && i + 1 < argc) { char *end; address = strtoul(argv[++i], &end, 0); if (*end || address > 0x7f) { fprintf(stderr, "Invalid I2C address\n"); return 2; } }
        else if (!strcmp(argv[i], "--dev") && i + 1 < argc) i2c_device = argv[++i];
        else if (!strcmp(argv[i], "--interface") && i + 1 < argc) interface = argv[++i];
        else if (!strcmp(argv[i], "--idle-dim") && i + 1 < argc) idle_dim = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--idle-blank") && i + 1 < argc) idle_blank = atoi(argv[++i]);
        else { fprintf(stderr, "Unknown or incomplete option: %s\n", argv[i]); usage(argv[0]); return 2; }
    }
    if (idle_dim < 0 || idle_blank < 0) { fprintf(stderr, "Idle times cannot be negative\n"); return 2; }
    signal(SIGINT, handle_signal); signal(SIGTERM, handle_signal);

    ssd1306_t display;
    if (ssd1306_init(&display, i2c_device, (uint8_t)address) != 0) return 1;
    if (gpio_buttons_init() != 0) fprintf(stderr, "[GPIO] Buttons unavailable\n");
    if (wifi_manager_init(interface) != 0) fprintf(stderr, "[WiFi] Background polling unavailable\n");

    char main_items[4][32] = {"Wi-Fi", "System", "RustyTune", "Shutdown"};
    char wifi_items[6][32] = {"Status", "Scan & Connect", "Saved Networks", "Switch to AP", "AP Settings", "Boot Preference"};
    char system_items[3][32] = {"System Information", "Reboot", "Debug Services"};
    char rustytune_items[8][32] = {"ECU / Service", "Connection", "Auto-log", "Pair Device", "Web Address", "Storage", "Diagnostics", "Restart Service"};
    wifi_network_t networks[WIFI_MAX_NETWORKS]; size_t network_count = 0, scan_selected = 0;
    char profiles[WIFI_MAX_PROFILES][WIFI_NAME_MAX]; size_t profile_count = 0, saved_selected = 0;
    char target_ssid[WIFI_NAME_MAX] = "", password[128] = "", ap_ssid[33] = "Pi-AP", ap_password[64] = "", dialog_title[22] = "", dialog_message[256] = "", dialog_footer[22] = "";
    app_state_t state = MAIN_MENU, dialog_return = MAIN_MENU; input_mode_t input_mode = INPUT_WIFI_PASSWORD;
    size_t main_selected = 0, wifi_selected=0, system_selected=0, rustytune_selected=0, rt_status_top=0, rt_connection_selected=0, boot_selected = 0, ap_settings_selected = 0; int osk_row = 0, osk_col = 0;
    rustytune_config_t rt_config={0}; char pairing_code[7]=""; unsigned pairing_expires=0; double pairing_started=0;
    bool keyboard_upper = false, ap_password_changed = false;
    button_state_t buttons[BTN_COUNT] = {{0}}; bool redraw = true, dimmed = false, blanked = false;
    double last_activity = now_seconds(), last_refresh = last_activity;

    while (running) {
        bool frame_drawn = false;
        double now = now_seconds(); bool activity = false; int button = triggered_button(buttons, now, &activity);
        if (activity) {
            bool was_asleep = dimmed || blanked; last_activity = now;
            if (was_asleep) { ssd1306_display_on(&display); ssd1306_set_contrast(&display, 255); dimmed = blanked = false; redraw = true; button = -1; }
        }
        double idle = now - last_activity;
        if (!blanked && idle_blank > 0 && idle >= idle_blank) { ssd1306_clear(&display); ssd1306_update(&display); ssd1306_display_off(&display); blanked = true; }
        else if (!blanked && !dimmed && idle_dim > 0 && idle >= idle_dim) { ssd1306_set_contrast(&display, 1); dimmed = true; }
        if (blanked) { usleep(50000); continue; }
        if (now - last_refresh >= 2.0) { last_refresh = now; redraw = true; }

        if (state == MAIN_MENU) {
            if (button == BTN_UP) { main_selected = (main_selected + 3) % 4; redraw = true; }
            else if (button == BTN_DOWN) { main_selected = (main_selected + 1) % 4; redraw = true; }
            else if (button == BTN_CENTER || button == BTN_5) {
                if (main_selected == 0) state = WIFI_MENU;
                else if (main_selected == 1) state = SYSTEM_MENU;
                else if (main_selected == 2) {
                    rustytune_status_t status;
                    if (rustytune_get_status(&status, dialog_message, sizeof(dialog_message)) == 0) {
                        state = RUSTYTUNE_MENU;
                    } else {
                        snprintf(dialog_title, sizeof(dialog_title), "RUSTYTUNE");
                        snprintf(dialog_footer, sizeof(dialog_footer), "#6: Back");
                        dialog_return = MAIN_MENU;
                        state = DIALOG;
                    }
                }
                else state = CONFIRM_SHUTDOWN;
                redraw = true;
            }
            if (redraw && state == MAIN_MENU) { wifi_status_t s; wifi_get_status(&s); render_menu(&display, "PI CONFIG", main_items, 4, main_selected, strstr(s.mode, "AP") ? "AP" : "WIFI"); frame_drawn = true; }
        } else if (state == WIFI_MENU) {
            if(button==BTN_6||button==BTN_LEFT){state=MAIN_MENU;redraw=true;}
            else if(button==BTN_UP){wifi_selected=(wifi_selected+5)%6;redraw=true;} else if(button==BTN_DOWN){wifi_selected=(wifi_selected+1)%6;redraw=true;}
            else if(button==BTN_CENTER||button==BTN_5){
                if(wifi_selected==0)state=WIFI_STATUS;
                else if(wifi_selected==1){
                    render_dialog(&display, "SCANNING", "Searching for Wi-Fi...", "Please wait..."); ssd1306_update(&display);
                    network_count = wifi_scan(networks, WIFI_MAX_NETWORKS, dialog_message, sizeof(dialog_message)); scan_selected = 0; state = WIFI_SCAN;
                } else if(wifi_selected==2){profile_count=wifi_get_saved_profiles(profiles,WIFI_MAX_PROFILES);saved_selected=0;state=WIFI_SAVED;}
                else if(wifi_selected==3){
                    wifi_status_t current;
                    wifi_get_status(&current);
                    if (strstr(current.mode, "AP")) {
                        state = CONFIRM_CLIENT;
                    } else {
                        render_dialog(&display, "AP MODE", "Starting AP hotspot...", "Please wait..."); ssd1306_update(&display);
                        int rc = wifi_enable_ap(dialog_message, sizeof(dialog_message));
                        snprintf(dialog_title, sizeof(dialog_title), "%s", rc == 0 ? "AP ACTIVE" : "AP ERROR");
                        snprintf(dialog_footer, sizeof(dialog_footer), "Press #6 to return"); dialog_return = WIFI_MENU; state = DIALOG;
                    }
                } else if(wifi_selected==4){wifi_ap_config_t config;wifi_get_ap_config(&config);snprintf(ap_ssid,sizeof(ap_ssid),"%.32s",config.ssid);ap_password[0]='\0';ap_password_changed=false;ap_settings_selected=0;state=AP_SETTINGS;}
                else {profile_count=wifi_get_saved_profiles(profiles,WIFI_MAX_PROFILES);boot_selected=0;state=BOOT_NETWORK;}
                redraw = true;
            }
            if(redraw&&state==WIFI_MENU){wifi_status_t s;wifi_get_status(&s);snprintf(wifi_items[3],sizeof(wifi_items[3]),"Switch to %s",strstr(s.mode,"AP")?"Client":"AP");render_menu(&display,"WI-FI",wifi_items,6,wifi_selected,NULL);frame_drawn=true;}
        } else if(state==SYSTEM_MENU){
            if(button==BTN_6||button==BTN_LEFT){state=MAIN_MENU;redraw=true;}else if(button==BTN_UP){system_selected=(system_selected+2)%3;redraw=true;}else if(button==BTN_DOWN){system_selected=(system_selected+1)%3;redraw=true;}else if(button==BTN_CENTER||button==BTN_5){if(system_selected==0)state=SYS_INFO;else if(system_selected==1)state=CONFIRM_REBOOT;else{snprintf(dialog_title,22,"DEBUG SERVICES");snprintf(dialog_message,sizeof(dialog_message),"Use journalctl over SSH");snprintf(dialog_footer,22,"#6: Back");dialog_return=SYSTEM_MENU;state=DIALOG;}redraw=true;}if(redraw&&state==SYSTEM_MENU){render_menu(&display,"SYSTEM",system_items,3,system_selected,NULL);frame_drawn=true;}
        } else if(state==RUSTYTUNE_MENU){
            if(button==BTN_6||button==BTN_LEFT){state=MAIN_MENU;redraw=true;}else if(button==BTN_UP){rustytune_selected=(rustytune_selected+7)%8;redraw=true;}else if(button==BTN_DOWN){rustytune_selected=(rustytune_selected+1)%8;redraw=true;}else if(button==BTN_CENTER||button==BTN_5){
                if(rustytune_selected==0){rt_status_top=0;state=RUSTYTUNE_STATUS;}
                else if(rustytune_selected==1){if(rustytune_get_config(&rt_config,dialog_message,sizeof(dialog_message))==0){rt_connection_selected=0;state=RUSTYTUNE_CONNECTION;}else{snprintf(dialog_title,22,"RUSTYTUNE");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}}
                else if(rustytune_selected==2){if(rustytune_get_config(&rt_config,dialog_message,sizeof(dialog_message))==0){rt_config.auto_log=!rt_config.auto_log;render_dialog(&display,"UPDATING","Saving auto-log...","Please wait...");ssd1306_update(&display);int rc=rustytune_update_connection(&rt_config,dialog_message,sizeof(dialog_message));if(rc==0)rc=rustytune_restart(dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"AUTO-LOG SAVED":"UPDATE FAILED");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}else{snprintf(dialog_title,22,"RUSTYTUNE");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}}
                else if(rustytune_selected==3){if(rustytune_open_pairing(pairing_code,&pairing_expires,dialog_message,sizeof(dialog_message))==0){pairing_started=now;state=RUSTYTUNE_PAIR;}else{snprintf(dialog_title,22,"PAIRING FAILED");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}}
                else if(rustytune_selected==4){wifi_status_t w;wifi_get_status(&w);snprintf(dialog_title,22,"WEB ADDRESS");snprintf(dialog_message,sizeof(dialog_message),"http://%.17s",w.ip);snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}
                else if(rustytune_selected==5)state=RUSTYTUNE_STORAGE;
                else if(rustytune_selected==6){render_dialog(&display,"RECONNECTING","Resetting ECU link...","Please wait...");ssd1306_update(&display);int rc=rustytune_reconnect(dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"RECONNECT REQUESTED":"RECONNECT FAILED");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}
                else {render_dialog(&display,"RESTARTING","Restarting service...","Please wait...");ssd1306_update(&display);int rc=rustytune_restart(dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"RESTART REQUESTED":"RESTART FAILED");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;}redraw=true;
            } if(redraw&&state==RUSTYTUNE_MENU){render_menu(&display,"RUSTYTUNE",rustytune_items,8,rustytune_selected,NULL);frame_drawn=true;}
        } else if(state==RUSTYTUNE_STATUS){
            if(button==BTN_6||button==BTN_LEFT){state=RUSTYTUNE_MENU;redraw=true;}
            else if(button==BTN_UP&&rt_status_top>0){rt_status_top--;redraw=true;}
            else if(button==BTN_DOWN&&rt_status_top<5){rt_status_top++;redraw=true;}
            if(redraw&&state==RUSTYTUNE_STATUS){rustytune_status_t s;char lines[9][32];if(rustytune_get_status(&s,dialog_message,sizeof(dialog_message))!=0){char card[2][22];snprintf(card[0],22,"%.20s",dialog_message);snprintf(card[1],22,"No changes made");render_card(&display,"RUSTYTUNE",card,2);}else{const char *device=strrchr(s.port,'/');snprintf(lines[0],32,"Service: running");snprintf(lines[1],32,"ECU: %s",rustytune_ecu_state(&s));snprintf(lines[2],32,"Device: %.20s",device?device+1:(s.port[0]?s.port:"closed"));snprintf(lines[3],32,"Link: %s @ %u",s.mode[0]?s.mode:"--",s.baud);snprintf(lines[4],32,"Frames: %llu",s.frames);snprintf(lines[5],32,"Timeout/CRC: %llu/%llu",s.timeouts,s.crc_errors);snprintf(lines[6],32,"Log: %s (%llu)",s.logging?"recording":"idle",s.log_rows);snprintf(lines[7],32,"Signature: %.19s",s.ecu_signature[0]?s.ecu_signature:"unknown");snprintf(lines[8],32,"Error: %.23s",s.error[0]?s.error:"none");render_info_list(&display,"ECU / SERVICE",lines,9,rt_status_top);}frame_drawn=true;}
        } else if(state==RUSTYTUNE_CONNECTION){
            char items[5][32];
            if(button==BTN_6||button==BTN_LEFT){state=RUSTYTUNE_MENU;redraw=true;}else if(button==BTN_UP){rt_connection_selected=(rt_connection_selected+4)%5;redraw=true;}else if(button==BTN_DOWN){rt_connection_selected=(rt_connection_selected+1)%5;redraw=true;}else if(button==BTN_CENTER||button==BTN_5){
                if(rt_connection_selected==0){detect_serial(rt_config.device,sizeof(rt_config.device));redraw=true;}
                else if(rt_connection_selected==1){snprintf(rt_config.mode,sizeof(rt_config.mode),"%s",strcmp(rt_config.mode,"secondary")?"secondary":"primary");redraw=true;}
                else if(rt_connection_selected==2){static const unsigned bauds[]={9600,19200,38400,57600,115200,230400};size_t i=0;while(i<5&&bauds[i]!=rt_config.baud)i++;rt_config.baud=bauds[(i+1)%6];redraw=true;}
                else if(rt_connection_selected==3){rt_config.auto_log=!rt_config.auto_log;redraw=true;}
                else {render_dialog(&display,"APPLYING","Saving and restart...","Please wait...");ssd1306_update(&display);int rc=rustytune_update_connection(&rt_config,dialog_message,sizeof(dialog_message));if(rc==0)rc=rustytune_restart(dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"SETTINGS APPLIED":"UPDATE FAILED");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;redraw=true;}
            }
            if(redraw&&state==RUSTYTUNE_CONNECTION){const char *base=strrchr(rt_config.device,'/');snprintf(items[0],32,"Device: %.19s",base?base+1:rt_config.device);snprintf(items[1],32,"Mode: %s",rt_config.mode);snprintf(items[2],32,"Baud: %u",rt_config.baud);snprintf(items[3],32,"Auto-log: %s",rt_config.auto_log?"On":"Off");snprintf(items[4],32,"Apply & Reconnect");render_menu(&display,"CONNECTION",items,5,rt_connection_selected,NULL);frame_drawn=true;}
        } else if(state==RUSTYTUNE_PAIR){
            if(button==BTN_6||button==BTN_LEFT){state=RUSTYTUNE_MENU;redraw=true;}
            unsigned elapsed=(unsigned)(now-pairing_started),remaining=elapsed<pairing_expires?pairing_expires-elapsed:0;if(!remaining){snprintf(dialog_title,22,"PAIRING EXPIRED");snprintf(dialog_message,sizeof(dialog_message),"Open a new window");snprintf(dialog_footer,22,"#6: Back");dialog_return=RUSTYTUNE_MENU;state=DIALOG;redraw=true;}else if(redraw&&state==RUSTYTUNE_PAIR){char lines[4][22];snprintf(lines[0],22,"Code: %s",pairing_code);snprintf(lines[1],22,"Expires: %u:%02u",(remaining/60)%100,remaining%60);snprintf(lines[2],22,"Open RustyTune web");snprintf(lines[3],22,"and enter code");render_card(&display,"PAIR DEVICE",lines,4);frame_drawn=true;}
        } else if(state==RUSTYTUNE_STORAGE){
            if(button==BTN_6||button==BTN_LEFT){state=RUSTYTUNE_MENU;redraw=true;}if(redraw&&state==RUSTYTUNE_STORAGE){char lines[4][22];struct statvfs fs;int storage_rc=statvfs("/var/log/speeduino",&fs);if(storage_rc==0){unsigned long long freeb=(unsigned long long)fs.f_bavail*fs.f_frsize;snprintf(lines[0],22,"Log directory ready");snprintf(lines[1],22,"Free: %.1f GiB",freeb/1073741824.0);if(rustytune_get_config(&rt_config,dialog_message,sizeof(dialog_message))==0)snprintf(lines[2],22,"Cap: %.1f GiB",rt_config.retention_bytes/1073741824.0);else snprintf(lines[2],22,"Cap: unavailable");snprintf(lines[3],22,"Oldest pruned first");}else{snprintf(lines[0],22,"Storage unavailable");snprintf(lines[1],22,"%.20s",strerror(errno));}render_card(&display,"LOG STORAGE",lines,storage_rc==0?4:2);frame_drawn=true;}
        } else if (state == WIFI_STATUS) {
            if (button == BTN_6 || button == BTN_LEFT) { state = WIFI_MENU; redraw = true; }
            if (redraw && state == WIFI_STATUS) { wifi_status_t s; wifi_get_status(&s); char lines[4][22]; snprintf(lines[0],22,"Mode: %.15s",s.mode); snprintf(lines[1],22,"SSID: %.15s",s.ssid); snprintf(lines[2],22,"IP: %.17s",s.ip); snprintf(lines[3],22,"Sig: %d%%",s.signal); render_card(&display,"WIFI STATUS",lines,4); frame_drawn = true; }
        } else if (state == WIFI_SCAN) {
            size_t count = network_count ? network_count : 1;
            if (button == BTN_6 || button == BTN_LEFT) { state = WIFI_MENU; redraw = true; }
            else if (button == BTN_UP) { scan_selected = (scan_selected + count - 1) % count; redraw = true; }
            else if (button == BTN_DOWN) { scan_selected = (scan_selected + 1) % count; redraw = true; }
            else if ((button == BTN_CENTER || button == BTN_5) && network_count) {
                snprintf(target_ssid,sizeof(target_ssid),"%s",networks[scan_selected].ssid);
                if (!strcmp(networks[scan_selected].security,"Open") || !strcmp(networks[scan_selected].security,"--")) {
                    int rc = wifi_connect(target_ssid,NULL,dialog_message,sizeof(dialog_message)); snprintf(dialog_title,22,"%s",rc==0?"CONNECTED":"FAILED"); snprintf(dialog_footer,22,"Press #6 to return"); dialog_return=WIFI_MENU; state=DIALOG;
                } else { password[0]='\0'; osk_row=osk_col=0; keyboard_upper=false; input_mode=INPUT_WIFI_PASSWORD; state=OSK_PASS; } redraw=true;
            }
            if (redraw && state == WIFI_SCAN) { char items[WIFI_MAX_NETWORKS][32]; if (!network_count) snprintf(items[0],32,"(No networks found)"); for(size_t i=0;i<network_count;i++){char ssid[21];ellipsize(ssid,sizeof(ssid),networks[i].ssid,18);snprintf(items[i],32,"%s (%d%%)",ssid,networks[i].signal);} render_menu(&display,"SCAN RESULTS",items,count,scan_selected,NULL); frame_drawn = true; }
        } else if (state == WIFI_SAVED) {
            size_t count = profile_count ? profile_count : 1;
            if (button == BTN_6 || button == BTN_LEFT) { state=WIFI_MENU; redraw=true; }
            else if(button==BTN_UP){saved_selected=(saved_selected+count-1)%count;redraw=true;} else if(button==BTN_DOWN){saved_selected=(saved_selected+1)%count;redraw=true;}
            else if((button==BTN_CENTER||button==BTN_5)&&profile_count){char connecting_message[64];snprintf(connecting_message,sizeof(connecting_message),"Activating %.49s...",profiles[saved_selected]);render_dialog(&display,"CONNECTING",connecting_message,"Please wait...");ssd1306_update(&display);int rc=wifi_connect_saved(profiles[saved_selected],dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"CONNECTED":"FAILED");snprintf(dialog_footer,22,"Press #6 to return");dialog_return=WIFI_MENU;state=DIALOG;redraw=true;}
            if(redraw&&state==WIFI_SAVED){char items[WIFI_MAX_PROFILES][32];if(!profile_count)snprintf(items[0],32,"(No saved profiles)");for(size_t i=0;i<profile_count;i++)ellipsize(items[i],32,profiles[i],19);render_menu(&display,"SAVED PROFILES",items,count,saved_selected,NULL);frame_drawn=true;}
        } else if (state == AP_SETTINGS) {
            if (button == BTN_6 || button == BTN_LEFT) { state = WIFI_MENU; redraw = true; }
            else if (button == BTN_UP) { ap_settings_selected = (ap_settings_selected + 3) % 4; redraw = true; }
            else if (button == BTN_DOWN) { ap_settings_selected = (ap_settings_selected + 1) % 4; redraw = true; }
            else if (button == BTN_CENTER || button == BTN_5) {
                if (ap_settings_selected == 0) {
                    snprintf(password, sizeof(password), "%s", ap_ssid); input_mode = INPUT_AP_SSID;
                    osk_row = osk_col = 0; keyboard_upper = false; state = OSK_PASS;
                } else if (ap_settings_selected == 1) {
                    password[0] = '\0'; input_mode = INPUT_AP_PASSWORD;
                    osk_row = osk_col = 0; keyboard_upper = false; state = OSK_PASS;
                } else if (ap_settings_selected == 2) {
                    render_dialog(&display, "SAVING AP", "Updating AP settings...", "Please wait..."); ssd1306_update(&display);
                    int rc = wifi_set_ap_config(ap_ssid, ap_password_changed ? ap_password : NULL,
                                                dialog_message, sizeof(dialog_message));
                    if (rc == 0) {
                        wifi_status_t current; wifi_get_status(&current);
                        if (strstr(current.mode, "AP")) state = CONFIRM_AP_RESTART;
                        else { snprintf(dialog_title, sizeof(dialog_title), "AP SETTINGS SAVED"); snprintf(dialog_footer, sizeof(dialog_footer), "Press #6 to return"); dialog_return = WIFI_MENU; state = DIALOG; }
                    } else { snprintf(dialog_title, sizeof(dialog_title), "AP SETTINGS ERROR"); snprintf(dialog_footer, sizeof(dialog_footer), "Press #6"); dialog_return = AP_SETTINGS; state = DIALOG; }
                } else state = CONFIRM_AP_RESET;
                redraw = true;
            }
            if (redraw && state == AP_SETTINGS) {
                char items[4][32], shown_ssid[22]; ellipsize(shown_ssid, sizeof(shown_ssid), ap_ssid, 18);
                snprintf(items[0], sizeof(items[0]), "SSID: %s", shown_ssid);
                snprintf(items[1], sizeof(items[1]), "Password: %s", ap_password_changed ? "changed" : "unchanged");
                snprintf(items[2], sizeof(items[2]), "Save Settings"); snprintf(items[3], sizeof(items[3]), "Reset Defaults");
                render_menu(&display, "AP SETTINGS", items, 4, ap_settings_selected, NULL); frame_drawn = true;
            }
        } else if (state == BOOT_NETWORK) {
            size_t count = profile_count + 2;
            if (button == BTN_6 || button == BTN_LEFT) { state = WIFI_MENU; redraw = true; }
            else if (button == BTN_UP) { boot_selected = (boot_selected + count - 1) % count; redraw = true; }
            else if (button == BTN_DOWN) { boot_selected = (boot_selected + 1) % count; redraw = true; }
            else if (button == BTN_CENTER || button == BTN_5) {
                render_dialog(&display, "SAVING DEFAULT", "Updating boot network...", "Please wait...");
                ssd1306_update(&display);
                wifi_boot_target_t target = boot_selected == 0 ? WIFI_BOOT_AP :
                                            (boot_selected == 1 ? WIFI_BOOT_AUTO_CLIENT : WIFI_BOOT_CLIENT_PROFILE);
                const char *profile = boot_selected >= 2 ? profiles[boot_selected - 2] : NULL;
                int rc = wifi_set_boot_default(target, profile, dialog_message, sizeof(dialog_message));
                snprintf(dialog_title, sizeof(dialog_title), "%s", rc == 0 ? "DEFAULT SAVED" : "DEFAULT ERROR");
                snprintf(dialog_footer, sizeof(dialog_footer), "Press #6 to return");
                dialog_return = WIFI_MENU;
                state = DIALOG;
                redraw = true;
            }
            if (redraw && state == BOOT_NETWORK) {
                char items[WIFI_MAX_PROFILES + 2][32];
                snprintf(items[0], sizeof(items[0]), "AP: Pi-AP");
                snprintf(items[1], sizeof(items[1]), "Auto Client");
                for (size_t i = 0; i < profile_count; ++i) {
                    char name[22];
                    ellipsize(name, sizeof(name), profiles[i], 18);
                    snprintf(items[i + 2], sizeof(items[i + 2]), "WiFi: %s", name);
                }
                render_menu(&display, "BOOT NETWORK", items, count, boot_selected, NULL);
                frame_drawn = true;
            }
        } else if (state == OSK_PASS) {
            if(button==BTN_6){state=input_mode==INPUT_WIFI_PASSWORD?WIFI_SCAN:AP_SETTINGS;redraw=true;} else if(button==BTN_UP){osk_row=(osk_row+5)%6;int cols=osk_row==5?7:8;if(osk_col>=cols)osk_col=cols-1;redraw=true;} else if(button==BTN_DOWN){osk_row=(osk_row+1)%6;int cols=osk_row==5?7:8;if(osk_col>=cols)osk_col=cols-1;redraw=true;} else if(button==BTN_LEFT){int cols=osk_row==5?7:8;osk_col=(osk_col+cols-1)%cols;redraw=true;} else if(button==BTN_RIGHT){int cols=osk_row==5?7:8;osk_col=(osk_col+1)%cols;redraw=true;}
            else if(button==BTN_CENTER||button==BTN_5){const char *key=keyboard[osk_row][osk_col];size_t len=strlen(password);if(!strcmp(key,"<")){if(len)password[len-1]='\0';}else if(!strcmp(key,"^")){keyboard_upper=!keyboard_upper;}else if(!strcmp(key,"OK")){if(input_mode==INPUT_WIFI_PASSWORD){render_dialog(&display,"CONNECTING","Joining network...","Please wait...");ssd1306_update(&display);int rc=wifi_connect(target_ssid,password,dialog_message,sizeof(dialog_message));snprintf(dialog_title,22,"%s",rc==0?"CONNECTED":"FAILED");snprintf(dialog_footer,22,"Press #6 to return");dialog_return=WIFI_MENU;state=DIALOG;}else if(input_mode==INPUT_AP_SSID){if(len>0&&len<=32){snprintf(ap_ssid,sizeof(ap_ssid),"%s",password);state=AP_SETTINGS;}}else{if(len>=8&&len<=63){snprintf(ap_password,sizeof(ap_password),"%s",password);ap_password_changed=true;state=AP_SETTINGS;}}}else{char value[2]={key[0],0};if(keyboard_upper&&isalpha((unsigned char)value[0]))value[0]=(char)toupper((unsigned char)value[0]);size_t limit=input_mode==INPUT_AP_SSID?32:(input_mode==INPUT_AP_PASSWORD?63:sizeof(password)-1);if(len<limit)strcat(password,value);}redraw=true;}
            if(redraw&&state==OSK_PASS){const char *title=input_mode==INPUT_AP_SSID?"SSID":"PASS";render_keyboard(&display,title,password,input_mode!=INPUT_AP_SSID,keyboard_upper,osk_col,osk_row);frame_drawn=true;}
        } else if (state == CONFIRM_AP_RESET) {
            if(button==BTN_6||button==BTN_LEFT){state=AP_SETTINGS;redraw=true;}
            else if(button==BTN_5||button==BTN_CENTER){render_dialog(&display,"RESETTING AP","Restoring defaults...","Please wait...");ssd1306_update(&display);int rc=wifi_set_ap_config("Pi-AP","raspberry",dialog_message,sizeof(dialog_message));if(rc==0){snprintf(ap_ssid,sizeof(ap_ssid),"Pi-AP");ap_password[0]='\0';ap_password_changed=false;wifi_status_t current;wifi_get_status(&current);if(strstr(current.mode,"AP"))state=CONFIRM_AP_RESTART;else{snprintf(dialog_title,sizeof(dialog_title),"DEFAULTS RESTORED");snprintf(dialog_footer,sizeof(dialog_footer),"Press #6 to return");dialog_return=WIFI_MENU;state=DIALOG;}}else{snprintf(dialog_title,sizeof(dialog_title),"RESET ERROR");snprintf(dialog_footer,sizeof(dialog_footer),"Press #6");dialog_return=AP_SETTINGS;state=DIALOG;}redraw=true;}
            if(redraw&&state==CONFIRM_AP_RESET){render_dialog(&display,"RESET AP?","Use Pi-AP defaults?","#5: Yes  #6: No");frame_drawn=true;}
        } else if (state == CONFIRM_AP_RESTART) {
            if(button==BTN_6||button==BTN_LEFT){state=AP_SETTINGS;redraw=true;}
            else if(button==BTN_5||button==BTN_CENTER){render_dialog(&display,"RESTARTING AP","Applying settings...","Please wait...");ssd1306_update(&display);int rc=wifi_enable_ap(dialog_message,sizeof(dialog_message));snprintf(dialog_title,sizeof(dialog_title),"%s",rc==0?"AP RESTARTED":"AP ERROR");snprintf(dialog_footer,sizeof(dialog_footer),"Press #6 to return");dialog_return=WIFI_MENU;state=DIALOG;redraw=true;}
            if(redraw&&state==CONFIRM_AP_RESTART){render_dialog(&display,"RESTART AP?","Apply settings now?","#5: Yes  #6: Later");frame_drawn=true;}
        } else if (state == CONFIRM_CLIENT) {
            if (button == BTN_6 || button == BTN_LEFT) { state = WIFI_MENU; redraw = true; }
            else if (button == BTN_5 || button == BTN_CENTER) {
                render_dialog(&display, "CLIENT MODE", "Reconnecting Wi-Fi...", "Please wait...");
                ssd1306_update(&display);
                int rc = wifi_disable_ap(dialog_message, sizeof(dialog_message));
                if (rc == 0) {
                    snprintf(dialog_title, sizeof(dialog_title), "CLIENT ACTIVE");
                    snprintf(dialog_footer, sizeof(dialog_footer), "Press #6 to return");
                    dialog_return = WIFI_MENU;
                } else {
                    profile_count = wifi_get_saved_profiles(profiles, WIFI_MAX_PROFILES);
                    saved_selected = 0;
                    snprintf(dialog_title, sizeof(dialog_title), "CLIENT ERROR");
                    snprintf(dialog_footer, sizeof(dialog_footer), "#5: Saved networks");
                    dialog_return = WIFI_SAVED;
                }
                state = DIALOG;
                redraw = true;
            }
            if (redraw && state == CONFIRM_CLIENT) { render_dialog(&display, "LEAVE AP MODE?", "Reconnect saved Wi-Fi?", "#5: Yes  #6: No"); frame_drawn = true; }
        } else if (state == CONFIRM_REBOOT) {
            if (button == BTN_6 || button == BTN_LEFT) { state = SYSTEM_MENU; redraw = true; }
            else if (button == BTN_5 || button == BTN_CENTER) {
                render_dialog(&display, "REBOOTING", "Requesting reboot...", "Please wait...");
                ssd1306_update(&display);
                int rc = system_request_reboot(dialog_message, sizeof(dialog_message));
                snprintf(dialog_title, sizeof(dialog_title), "%s", rc == 0 ? "REBOOTING" : "REBOOT ERROR");
                snprintf(dialog_footer, sizeof(dialog_footer), "%s", rc == 0 ? "Please wait..." : "Press #6");
                dialog_return = SYSTEM_MENU;
                state = DIALOG;
                redraw = true;
            }
            if (redraw && state == CONFIRM_REBOOT) { render_dialog(&display, "REBOOT PI?", "Restart the system?", "#5: Yes  #6: No"); frame_drawn = true; }
        } else if (state == CONFIRM_SHUTDOWN) {
            if (button == BTN_6 || button == BTN_LEFT) { state = MAIN_MENU; redraw = true; }
            else if (button == BTN_5 || button == BTN_CENTER) {
                render_dialog(&display, "SHUTTING DOWN", "Requesting shutdown...", "Please wait...");
                ssd1306_update(&display);
                int rc = system_request_shutdown(dialog_message, sizeof(dialog_message));
                snprintf(dialog_title, sizeof(dialog_title), "%s", rc == 0 ? "SHUTTING DOWN" : "SHUTDOWN ERROR");
                snprintf(dialog_footer, sizeof(dialog_footer), "%s", rc == 0 ? "Please wait..." : "Press #6");
                dialog_return = MAIN_MENU;
                state = DIALOG;
                redraw = true;
            }
            if (redraw && state == CONFIRM_SHUTDOWN) { render_dialog(&display, "SHUTDOWN PI?", "Power off the system?", "#5: Yes  #6: No"); frame_drawn = true; }
        } else if (state == DIALOG) {
            if(button==BTN_6||button==BTN_5||button==BTN_CENTER){state=dialog_return;redraw=true;}
            if(redraw&&state==DIALOG){render_dialog(&display,dialog_title,dialog_message,dialog_footer);frame_drawn=true;}
        } else if (state == SYS_INFO) {
            if(button==BTN_6||button==BTN_LEFT){state=SYSTEM_MENU;redraw=true;}
            if(redraw&&state==SYS_INFO){sys_info_t s;sys_get_info(&s);char lines[4][22];snprintf(lines[0],22,"Host: %.15s",s.hostname);snprintf(lines[1],22,"IP: %.17s",s.ip_wlan);snprintf(lines[2],22,"Temp: %.15s",s.temp);snprintf(lines[3],22,"RAM: %.16s",s.ram);render_card(&display,"SYSTEM INFO",lines,4);frame_drawn=true;}
        }
        if(frame_drawn){ssd1306_update(&display);redraw=false;} usleep(10000);
    }
    wifi_manager_close(); gpio_buttons_close(); ssd1306_close(&display); return 0;
}
