#ifndef SYSTEM_CONTROL_H
#define SYSTEM_CONTROL_H

#include <stddef.h>

int system_request_reboot(char *message, size_t message_size);
int system_request_shutdown(char *message, size_t message_size);

#endif
