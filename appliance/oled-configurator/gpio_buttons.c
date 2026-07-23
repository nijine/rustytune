#include "gpio_buttons.h"
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

#ifdef HAVE_LIBGPIOD
#include <gpiod.h>
static struct gpiod_chip *chip = NULL;
static struct gpiod_line *lines[BTN_COUNT];
#else
static int gpio_fds[BTN_COUNT] = {-1, -1, -1, -1, -1, -1, -1};
#endif

static const int bcm_pins[BTN_COUNT] = {
    17, // BTN_UP
    22, // BTN_DOWN
    27, // BTN_LEFT
    23, // BTN_RIGHT
    4,  // BTN_CENTER
    5,  // BTN_5
    6   // BTN_6
};

int gpio_buttons_init(void) {
#ifdef HAVE_LIBGPIOD
    chip = gpiod_chip_open_by_number(0);
    if (!chip) chip = gpiod_chip_open_by_name("gpiochip4"); // RPi 5 fallback
    if (!chip) chip = gpiod_chip_open_by_name("pinctrl-bcm2835");

    if (!chip) {
        perror("[GPIO] gpiod_chip_open failed");
        return -1;
    }

    for (int i = 0; i < BTN_COUNT; i++) {
        lines[i] = gpiod_chip_get_line(chip, bcm_pins[i]);
        if (lines[i]) {
            // Bonnet buttons connect their GPIO to ground when pressed. Keep
            // released inputs at a defined HIGH level instead of allowing
            // them to float and generate phantom presses.
            gpiod_line_request_input_flags(
                lines[i],
                "bonnet-buttons",
                GPIOD_LINE_REQUEST_FLAG_BIAS_PULL_UP
            );
        }
    }
    return 0;
#else
    // Sysfs GPIO fallback for universal compatibility
    for (int i = 0; i < BTN_COUNT; i++) {
        char path[64];
        snprintf(path, sizeof(path), "/sys/class/gpio/gpio%d/value", bcm_pins[i]);
        gpio_fds[i] = open(path, O_RDONLY | O_NONBLOCK);
        if (gpio_fds[i] < 0) {
            // Try exporting pin
            int exp_fd = open("/sys/class/gpio/export", O_WRONLY);
            if (exp_fd >= 0) {
                char pin_str[16];
                snprintf(pin_str, sizeof(pin_str), "%d", bcm_pins[i]);
                write(exp_fd, pin_str, strlen(pin_str));
                close(exp_fd);
            }
            gpio_fds[i] = open(path, O_RDONLY | O_NONBLOCK);
        }
    }
    return 0;
#endif
}

void gpio_buttons_close(void) {
#ifdef HAVE_LIBGPIOD
    for (int i = 0; i < BTN_COUNT; i++) {
        if (lines[i]) gpiod_line_release(lines[i]);
    }
    if (chip) gpiod_chip_close(chip);
#else
    for (int i = 0; i < BTN_COUNT; i++) {
        if (gpio_fds[i] >= 0) close(gpio_fds[i]);
    }
#endif
}

bool gpio_button_is_pressed(button_id_t btn) {
    if (btn < 0 || btn >= BTN_COUNT) return false;

#ifdef HAVE_LIBGPIOD
    if (!lines[btn]) return false;
    int val = gpiod_line_get_value(lines[btn]);
    return (val == 0); // Active LOW
#else
    int fd = gpio_fds[btn];
    if (fd < 0) return false;

    char buf[4] = {0};
    lseek(fd, 0, SEEK_SET);
    if (read(fd, buf, sizeof(buf) - 1) > 0) {
        return (buf[0] == '0'); // Active LOW
    }
    return false;
#endif
}

bool gpio_any_button_pressed(void) {
    for (int i = 0; i < BTN_COUNT; i++) {
        if (gpio_button_is_pressed((button_id_t)i)) return true;
    }
    return false;
}
