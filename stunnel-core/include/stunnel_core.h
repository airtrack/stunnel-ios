#ifndef RUST_CORE_H
#define RUST_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

void stunnel_init_logging(void);

void* stunnel_create(const char* config_json);

bool stunnel_start(void* handle);

void stunnel_stop(void* handle);

void stunnel_process_packet(void* handle, const uint8_t* packet, size_t len);

typedef void (*packet_callback_t)(void* context, const uint8_t* packet, size_t len);

void stunnel_set_packet_callback(void* handle, void* context, packet_callback_t callback);

void stunnel_clear_packet_callback(void* handle);

#endif
