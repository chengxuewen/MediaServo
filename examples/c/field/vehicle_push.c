/* Vehicle-side push example (C ABI consumption) — mediaservo_field_* usage.
 *
 * Build (linking libmediaservo_field.so):
 *   gcc vehicle_push.c -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
 *       -L target/debug -lmediaservo_field -o vehicle_push
 * Run: LD_LIBRARY_PATH=target/debug ./vehicle_push
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "mediaservo/field.h"

int main(void) {
        /* 1. Configure */
    mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;
    cfg.url = "ws://127.0.0.1:9800/ws";
    cfg.psk = "mediaservo-dev";
    cfg.room = "vehicle-c";
    cfg.width = 1280;
    cfg.height = 720;
    cfg.framerate = 30;
    cfg.bitrate_kbps = 2000;

    char err[256];

        /* 2. Connect */
    mediaservo_field_push_t* s = NULL;
    mediaservo_err_t rc = mediaservo_field_push_connect(&cfg, &s);
    if (rc != MEDIASERVO_OK) {
        mediaservo_field_last_error(err, sizeof(err));
        fprintf(stderr, "connect failed (%d): %s\n", rc, err);
        return 1;
    }
    printf("connected\n");

        /* 3. Publish the video track */
    char track[64];
    rc = mediaservo_field_push_publish_video(s, track, sizeof(track));
    if (rc != MEDIASERVO_OK) {
        mediaservo_field_last_error(err, sizeof(err));
        fprintf(stderr, "publish failed (%d): %s\n", rc, err);
        mediaservo_field_push_close(s);
        return 1;
    }
    printf("published: track=%s\n", track);

        /* 4. Start frame generation */
    rc = mediaservo_field_push_start_video_frames(s);
    if (rc != MEDIASERVO_OK) {
        mediaservo_field_last_error(err, sizeof(err));
        fprintf(stderr, "start frames failed (%d): %s\n", rc, err);
        mediaservo_field_push_close(s);
        return 1;
    }
    printf("frames running\n");

        /* 5. Run for 30s (vehicle-side loop) */
    for (int i = 0; i < 30; i++) {
        sleep(1);
        if (i % 10 == 0) printf("t=%ds ...\n", i);
    }

        /* 6. Stop + close */
    mediaservo_field_push_stop_video_frames(s);
    rc = mediaservo_field_push_close(s);
    if (rc != MEDIASERVO_OK) {
        mediaservo_field_last_error(err, sizeof(err));
        fprintf(stderr, "close failed (%d): %s\n", rc, err);
        return 1;
    }
    printf("done\n");
    return 0;
}
