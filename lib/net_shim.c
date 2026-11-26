/* TCP client shim for lib/net.loi.
 *
 * Build a Loi program that imports the library with:
 *   loi build main.loi -o main --c-source lib/net_shim.c
 *
 * The declarations mirror the `extern flow`s in lib/net.loi. Sockets are
 * exposed as integer descriptors; the Loi-side slice struct is repeated here
 * because slices are an FFI-only shape in generated C.
 */
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif

#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

int32_t loi_net_tcp_connect(const char* host, uint32_t port) {
    if (host == NULL || port == 0 || port > 65535) {
        return -1;
    }
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        return -1;
    }
    struct sockaddr_in address;
    memset(&address, 0, sizeof(address));
    address.sin_family = AF_INET;
    address.sin_port = htons((uint16_t)port);
    if (inet_pton(AF_INET, host, &address.sin_addr) != 1) {
        close(fd);
        return -1;
    }
    if (connect(fd, (struct sockaddr*)&address, sizeof(address)) != 0) {
        close(fd);
        return -1;
    }
    return (int32_t)fd;
}

int64_t loi_net_send(int32_t fd, LoiSlice data) {
    if (fd < 0 || (data.len > 0 && data.data == NULL)) {
        return -1;
    }
    ssize_t written = send((int)fd, data.data, data.len, 0);
    return (int64_t)written;
}

int64_t loi_net_recv(int32_t fd, LoiSlice* data) {
    if (fd < 0 || data == NULL || (data->len > 0 && data->data == NULL)) {
        return -1;
    }
    ssize_t received = recv((int)fd, data->data, data->len, 0);
    return (int64_t)received;
}
