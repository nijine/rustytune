#define _POSIX_C_SOURCE 200809L
#include "system_control.h"

#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static int system_request(const char *action, const char *label,
                          char *message, size_t message_size) {
    pid_t pid = fork();
    if (pid == 0) {
        execlp("sudo", "sudo", "-n", "systemctl", action, (char *)NULL);
        _exit(127);
    }
    if (pid < 0) {
        snprintf(message, message_size, "%s failed: %s", label, strerror(errno));
        return -1;
    }

    int status = 0;
    while (waitpid(pid, &status, 0) < 0) {
        if (errno == EINTR) continue;
        snprintf(message, message_size, "%s failed: %s", label, strerror(errno));
        return -1;
    }
    if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
        snprintf(message, message_size, "%s requested", label);
        return 0;
    }
    snprintf(message, message_size, "systemctl %s failed", action);
    return -1;
}

int system_request_reboot(char *message, size_t message_size) {
    return system_request("reboot", "Reboot", message, message_size);
}

int system_request_shutdown(char *message, size_t message_size) {
    return system_request("poweroff", "Shutdown", message, message_size);
}
