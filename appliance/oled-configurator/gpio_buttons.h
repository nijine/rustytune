#ifndef GPIO_BUTTONS_H
#define GPIO_BUTTONS_H

#include <stdbool.h>

typedef enum {
    BTN_UP = 0,
    BTN_DOWN,
    BTN_LEFT,
    BTN_RIGHT,
    BTN_CENTER,
    BTN_5,
    BTN_6,
    BTN_COUNT
} button_id_t;

int gpio_buttons_init(void);
void gpio_buttons_close(void);
bool gpio_button_is_pressed(button_id_t btn);
bool gpio_any_button_pressed(void);

#endif // GPIO_BUTTONS_H
