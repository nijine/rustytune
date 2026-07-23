#include "ssd1306.h"
#include "font5x7.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

#ifndef I2C_SLAVE
#define I2C_SLAVE 0x0703
#endif

static int i2c_write_cmd(ssd1306_t *dev, uint8_t cmd) {
    if (dev->i2c_fd < 0) return -1;
    uint8_t buf[2] = {0x00, cmd};
    return write(dev->i2c_fd, buf, 2) == 2 ? 0 : -1;
}

int ssd1306_init(ssd1306_t *dev, const char *i2c_dev, uint8_t addr) {
    memset(dev, 0, sizeof(ssd1306_t));
    dev->addr = addr;
    dev->i2c_fd = open(i2c_dev, O_RDWR);

    if (dev->i2c_fd < 0) {
        perror("[SSD1306] Failed to open I2C bus device");
        return -1;
    }

    if (ioctl(dev->i2c_fd, I2C_SLAVE, addr) < 0) {
        perror("[SSD1306] Failed to set I2C slave address");
        close(dev->i2c_fd);
        dev->i2c_fd = -1;
        return -1;
    }

    // SSD1306 128x64 Initialization Sequence
    static const uint8_t init_cmds[] = {
        0xAE,       // Display OFF
        0xD5, 0x80, // Display Clock Divide Ratio
        0xA8, 0x3F, // Multiplex Ratio (63)
        0xD3, 0x00, // Display Offset
        0x40,       // Start Line (0)
        0x8D, 0x14, // Charge Pump Enable
        0x20, 0x00, // Horizontal Memory Addressing Mode
        0xA1,       // Segment Re-map (col 127 mapped to SEG0)
        0xC8,       // COM Output Scan Direction (remapped)
        0xDA, 0x12, // COM Pins Hardware Config
        0x81, 0xCF, // Contrast Control
        0xD9, 0xF1, // Pre-charge Period
        0xDB, 0x40, // VCOMH Deselect Level
        0xA4,       // Entire Display ON (RAM)
        0xA6,       // Normal Display (not inverted)
        0xAF        // Display ON
    };

    for (size_t i = 0; i < sizeof(init_cmds); i++) {
        i2c_write_cmd(dev, init_cmds[i]);
    }

    ssd1306_clear(dev);
    ssd1306_update(dev);
    return 0;
}

void ssd1306_close(ssd1306_t *dev) {
    if (dev->i2c_fd >= 0) {
        ssd1306_clear(dev);
        ssd1306_update(dev);
        ssd1306_display_off(dev);
        close(dev->i2c_fd);
        dev->i2c_fd = -1;
    }
}

void ssd1306_clear(ssd1306_t *dev) {
    memset(dev->buffer, 0, SSD1306_BUF_SIZE);
}

void ssd1306_update(ssd1306_t *dev) {
    if (dev->i2c_fd < 0) return;

    // Set Column Address (0..127)
    i2c_write_cmd(dev, 0x21);
    i2c_write_cmd(dev, 0);
    i2c_write_cmd(dev, SSD1306_WIDTH - 1);

    // Set Page Address (0..7)
    i2c_write_cmd(dev, 0x22);
    i2c_write_cmd(dev, 0);
    i2c_write_cmd(dev, (SSD1306_HEIGHT / 8) - 1);

    // Send 1024 bytes buffer preceded by control byte 0x40 (Co=0, D/C#=1)
    uint8_t data_pkt[SSD1306_BUF_SIZE + 1];
    data_pkt[0] = 0x40;
    memcpy(data_pkt + 1, dev->buffer, SSD1306_BUF_SIZE);

    if (write(dev->i2c_fd, data_pkt, sizeof(data_pkt)) != (ssize_t)sizeof(data_pkt)) {
        // Output write error if needed
    }
}

void ssd1306_draw_pixel(ssd1306_t *dev, int x, int y, uint8_t color) {
    if (x < 0 || x >= SSD1306_WIDTH || y < 0 || y >= SSD1306_HEIGHT) return;

    int idx = x + (y / 8) * SSD1306_WIDTH;
    uint8_t bit = 1 << (y % 8);

    if (color) {
        dev->buffer[idx] |= bit;
    } else {
        dev->buffer[idx] &= ~bit;
    }
}

void ssd1306_draw_rect(ssd1306_t *dev, int x, int y, int w, int h, uint8_t color, bool fill) {
    if (fill) {
        for (int i = x; i < x + w; i++) {
            for (int j = y; j < y + h; j++) {
                ssd1306_draw_pixel(dev, i, j, color);
            }
        }
    } else {
        for (int i = x; i < x + w; i++) {
            ssd1306_draw_pixel(dev, i, y, color);
            ssd1306_draw_pixel(dev, i, y + h - 1, color);
        }
        for (int j = y; j < y + h; j++) {
            ssd1306_draw_pixel(dev, x, j, color);
            ssd1306_draw_pixel(dev, x + w - 1, j, color);
        }
    }
}

void ssd1306_draw_line(ssd1306_t *dev, int x0, int y0, int x1, int y1, uint8_t color) {
    int dx = abs(x1 - x0), sx = x0 < x1 ? 1 : -1;
    int dy = -abs(y1 - y0), sy = y0 < y1 ? 1 : -1;
    int err = dx + dy, e2;

    while (1) {
        ssd1306_draw_pixel(dev, x0, y0, color);
        if (x0 == x1 && y0 == y1) break;
        e2 = 2 * err;
        if (e2 >= dy) { err += dy; x0 += sx; }
        if (e2 <= dx) { err += dx; y0 += sy; }
    }
}

void ssd1306_draw_char(ssd1306_t *dev, int x, int y, char c, uint8_t color, uint8_t bg) {
    if (c < 32 || c > 126) c = '?';
    int font_idx = c - 32;

    for (int col = 0; col < 5; col++) {
        uint8_t line = font5x7[font_idx][col];
        for (int row = 0; row < 7; row++) {
            uint8_t pixel_color = (line & (1 << row)) ? color : bg;
            ssd1306_draw_pixel(dev, x + col, y + row, pixel_color);
        }
    }
    // Draw 1-pixel gap after character
    for (int row = 0; row < 7; row++) {
        ssd1306_draw_pixel(dev, x + 5, y + row, bg);
    }
}

void ssd1306_draw_string(ssd1306_t *dev, int x, int y, const char *str, uint8_t color, uint8_t bg) {
    while (*str) {
        ssd1306_draw_char(dev, x, y, *str, color, bg);
        x += 6; // 5 pixels width + 1 pixel spacing
        if (x >= SSD1306_WIDTH) break;
        str++;
    }
}

void ssd1306_set_contrast(ssd1306_t *dev, uint8_t contrast) {
    i2c_write_cmd(dev, 0x81);
    i2c_write_cmd(dev, contrast);
}

void ssd1306_display_on(ssd1306_t *dev) {
    i2c_write_cmd(dev, 0xAF);
}

void ssd1306_display_off(ssd1306_t *dev) {
    i2c_write_cmd(dev, 0xAE);
}
