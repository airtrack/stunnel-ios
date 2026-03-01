#ifndef RUST_CORE_H
#define RUST_CORE_H

#include <stddef.h>
#include <stdint.h>

void stunnel_init_logging(void);

void* stunnel_start(const char* config_json);

void stunnel_stop(void* handle);

void stunnel_process_packet(void* handle, const uint8_t* packet, size_t len);

typedef void (*packet_callback_t)(const uint8_t* packet, size_t len);

void stunnel_set_packet_callback(void* handle, packet_callback_t callback);

#endif
