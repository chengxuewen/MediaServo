/* Vehicle-side signaling example (C ABI consumption) — mediaservo_link_signal_* usage.
 *
 * Build (linking libmediaservo_link.so):
 *   gcc vehicle_signal.c -I bindings/c/mediaservo-link-c/include -I bindings/c/include \
 *       -L target/debug -lmediaservo_link -o vehicle_signal
 * Run: LD_LIBRARY_PATH=target/debug ./vehicle_signal
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "mediaservo/link.h"

static void on_event(mediaservo_link_signal_t* s, const char* event_json, void* user) {
    (void)s;
    (void)user;
    printf("event: %s\n", event_json);
}

int main(void) {
        /* 1. Configure */
    mediaservo_link_signal_config_t cfg = MEDIASERVO_LINK_SIGNAL_CONFIG_DEFAULT;
    cfg.url = "ws://127.0.0.1:9800/ws";
    cfg.psk = "mediaservo-dev";
    cfg.room = "vehicle-link-c";
    cfg.role = "Host";

    char err[256];

        /* 2. Connect */
    mediaservo_link_signal_t* s = NULL;
    mediaservo_err_t rc = mediaservo_link_signal_connect(&cfg, &s);
    if (rc != MEDIASERVO_OK) {
        mediaservo_link_last_error(err, sizeof(err));
        fprintf(stderr, "connect failed (%d): %s\n", rc, err);
        return 1;
    }
    printf("connected\n");

        /* 3. Register the event callback (pump thread: synthesize Connected + forward events) */
    mediaservo_link_signal_on_event(s, on_event, NULL);

        /* 4. Send one EncoderStatus (the server broadcasts/relays it to the room) */
    const char* status =
        "{\"type\":\"encoder_status\",\"room_id\":\"vehicle-link-c\","
        "\"peer_id\":\"vehicle-link-c\",\"codec\":\"h264\","
        "\"encoder_backend\":\"hardware\",\"frames_per_second\":30.0,"
        "\"frame_width\":1280,\"frame_height\":720}";
    rc = mediaservo_link_signal_send(s, status, strlen(status));
    if (rc != MEDIASERVO_OK) {
        mediaservo_link_last_error(err, sizeof(err));
        fprintf(stderr, "send failed (%d): %s\n", rc, err);
    } else {
        printf("sent encoder_status\n");
    }

        /* 5. Run for 10s (vehicle-side loop) */
    for (int i = 0; i < 10; i++) {
        sleep(1);
    }

        /* 6. Close */
    rc = mediaservo_link_signal_close(s);
    if (rc != MEDIASERVO_OK) {
        mediaservo_link_last_error(err, sizeof(err));
        fprintf(stderr, "close failed (%d): %s\n", rc, err);
        return 1;
    }
    printf("done\n");
    return 0;
}
