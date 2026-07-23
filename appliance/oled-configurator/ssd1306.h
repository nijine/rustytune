#ifndef SSD1306_H
#define SSD1306_H

#include <stdint.h>
#include <stdbool.h>

#define SSD1306_WIDTH  128
#define SSD1306_HEIGHT 64
#define SSD1306_BUF_SIZE ((SSD1306_WIDTH * SSD1306_HEIGHT) / 8)

typedef struct {
    int i2c_fd;
    uint8_t addr;
    uint8_t buffer[SSD1306_BUF_SIZE];
} ssd1306_t;

int ssd1306_init(ssd1306_t *dev, const char *i2c_dev, uint8_t addr);
void ssd1306_close(ssd1306_t *dev);
void ssd1306_clear(ssd1306_t *dev);
void ssd1306_update(ssd1306_t *dev);

void ssd1306_draw_pixel(ssd1306_t *dev, int x, int y, uint8_t color);
void ssd1306_draw_rect(ssd1306_t *dev, int x, int y, int w, int h, uint8_t color, bool fill);
void ssd1306_draw_line(ssd1306_t *dev, int x0, int y0, int x1, int y1, uint8_t color);
void ssd1306_draw_char(ssd1306_t *dev, int x, int y, char c, uint8_t color, uint8_t bg);
void ssd1306_draw_string(ssd1306_t *dev, int x, int y, const char *str, uint8_t color, uint8_t bg);

void ssd1306_set_contrast(ssd1306_t *dev, uint8_t contrast);
void ssd1306_display_on(ssd1306_t *dev);
void ssd1306_display_off(ssd1306_t *dev);

#endif // SSD1306_H
