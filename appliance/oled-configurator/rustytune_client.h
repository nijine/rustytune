#ifndef RUSTYTUNE_CLIENT_H
#define RUSTYTUNE_CLIENT_H

#include <stdbool.h>
#include <stddef.h>

#define RUSTYTUNE_ADMIN_SOCKET "/run/rustytune/admin.sock"

typedef struct {
    bool installed;
    bool connected;
    bool logging;
    char port[96];
    char mode[16];
    unsigned baud;
    unsigned long long frames;
    unsigned long long timeouts;
    unsigned long long crc_errors;
    unsigned long long log_rows;
    char ecu_signature[64];
    char error[128];
} rustytune_status_t;

typedef struct {
    char device[96];
    char mode[16];
    unsigned baud;
    bool auto_log;
    unsigned long long retention_bytes;
} rustytune_config_t;

int rustytune_get_status(rustytune_status_t *status, char *message, size_t size);
int rustytune_get_config(rustytune_config_t *config, char *message, size_t size);
int rustytune_open_pairing(char code[7], unsigned *expires_in, char *message, size_t size);
int rustytune_reconnect(char *message, size_t size);
int rustytune_restart(char *message, size_t size);
int rustytune_update_connection(const rustytune_config_t *config, char *message, size_t size);

#endif
