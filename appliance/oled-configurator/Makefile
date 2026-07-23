CC ?= cc
CFLAGS ?= -O3
CFLAGS += -std=c11 -Wall -Wextra -Wpedantic -pthread
LDFLAGS += -pthread

# Raspberry Pi OS currently ships the libgpiod v1 API. The sysfs fallback is
# retained so the sources can still compile on development machines.
ifneq ($(shell pkg-config --exists libgpiod && echo yes),)
CFLAGS += -DHAVE_LIBGPIOD $(shell pkg-config --cflags libgpiod)
LDFLAGS += $(shell pkg-config --libs libgpiod)
endif

TARGET = pi_configurator_c
SRCS = main.c ssd1306.c gpio_buttons.c wifi_manager.c system_control.c rustytune_client.c
OBJS = $(SRCS:.c=.o)

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) $(CFLAGS) -o $@ $(OBJS) $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJS) $(TARGET)

check: $(TARGET)
	./$(TARGET) --help >/dev/null
	grep -qx 'dhcp-option=3' rustytune-ap-dnsmasq.conf
	grep -qx 'dhcp-option=6' rustytune-ap-dnsmasq.conf

.PHONY: all clean check
