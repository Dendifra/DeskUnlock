#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include <wayland-client.h>

#include "ext-idle-notify-v1-client-protocol.h"

typedef struct {
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_seat *seat;
    struct ext_idle_notifier_v1 *notifier;
    struct ext_idle_notification_v1 *notification;
    struct ext_idle_notification_v1 *input_notification;
    bool locked;
    uint32_t timeout_ms;
} IdleClient;

static void lock_session(IdleClient *client) {
    if (client->locked) {
        return;
    }
    client->locked = true;
    fprintf(stderr, "syauth-idle-lock: idled after %u ms — locking session via loginctl\n", client->timeout_ms);

    pid_t child = fork();
    if (child < 0) {
        perror("syauth-idle-lock: fork");
        client->locked = false;
        return;
    }
    if (child == 0) {
        const char *session = getenv("XDG_SESSION_ID");
        if (session && *session) {
            execlp("loginctl", "loginctl", "lock-session", session, (char *)NULL);
        } else {
            execlp("loginctl", "loginctl", "lock-session", (char *)NULL);
        }
        _exit(127);
    }

    int status = 0;
    while (waitpid(child, &status, 0) < 0 && errno == EINTR) {
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        fprintf(stderr, "syauth-idle-lock: session lock request failed\n");
        client->locked = false;
    } else {
        fprintf(stderr, "syauth-idle-lock: session lock request accepted by loginctl\n");
    }
}

static void idle_idled(void *data, struct ext_idle_notification_v1 *notification) {
    (void)notification;
    fprintf(stderr, "syauth-idle-lock: idled event received (normal notification)\n");
    lock_session(data);
}

static void input_idle_idled(void *data, struct ext_idle_notification_v1 *notification) {
    (void)notification;
    fprintf(stderr, "syauth-idle-lock: idled event received (INPUT notification, inhibitors ignored)\n");
    lock_session(data);
}

static void idle_resumed(void *data, struct ext_idle_notification_v1 *notification) {
    IdleClient *client = data;
    (void)notification;
    fprintf(stderr, "syauth-idle-lock: resumed event received\n");
    client->locked = false;
}

static const struct ext_idle_notification_v1_listener idle_listener = {
    .idled = idle_idled,
    .resumed = idle_resumed,
};

static const struct ext_idle_notification_v1_listener input_idle_listener = {
    .idled = input_idle_idled,
    .resumed = idle_resumed,
};

static void registry_global(void *data, struct wl_registry *registry, uint32_t name,
                            const char *interface, uint32_t version) {
    IdleClient *client = data;
    if (strcmp(interface, "wl_seat") == 0 && !client->seat) {
        client->seat = wl_registry_bind(registry, name, &wl_seat_interface, version < 1 ? version : 1);
    } else if (strcmp(interface, "ext_idle_notifier_v1") == 0 && !client->notifier) {
        client->notifier = wl_registry_bind(registry, name, &ext_idle_notifier_v1_interface, version < 2 ? version : 2);
    }
}

static void registry_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_global_remove,
};

static int read_timeout_ms(const char *path, uint32_t *timeout_ms) {
    FILE *file = fopen(path, "r");
    if (!file) {
        perror("syauth-idle-lock: config");
        return 1;
    }
    char line[128];
    unsigned enabled = 0;
    unsigned minutes = 10;
    while (fgets(line, sizeof(line), file)) {
        unsigned value;
        if (sscanf(line, "idle_lock_enabled=%u", &value) == 1) {
            enabled = value;
        } else if (sscanf(line, "idle_lock_minutes=%u", &value) == 1) {
            minutes = value;
        }
    }
    fclose(file);
    if (!enabled) {
        return 2;
    }
    if (minutes < 1 || minutes > 120) {
        fprintf(stderr, "syauth-idle-lock: invalid timeout\n");
        return 1;
    }
    *timeout_ms = minutes * 60u * 1000u;
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 2 || strcmp(argv[1], "run") != 0) {
        fprintf(stderr, "usage: syauth-idle-lock-bin run\n");
        return 2;
    }
    const char *config = getenv("SYAUTH_IDLE_CONFIG");
    if (!config || !*config) {
        const char *home = getenv("HOME");
        if (!home) {
            fprintf(stderr, "syauth-idle-lock: HOME is unset\n");
            return 1;
        }
        static char fallback[4096];
        snprintf(fallback, sizeof(fallback), "%s/.config/syauth/idle.conf", home);
        config = fallback;
    }

    IdleClient client = {0};
    int config_status = read_timeout_ms(config, &client.timeout_ms);
    if (config_status == 2) {
        return 0;
    }
    if (config_status != 0) {
        return 1;
    }

    client.display = wl_display_connect(NULL);
    if (!client.display) {
        fprintf(stderr, "syauth-idle-lock: Wayland display unavailable\n");
        return 1;
    }
    client.registry = wl_display_get_registry(client.display);
    wl_registry_add_listener(client.registry, &registry_listener, &client);
    if (wl_display_roundtrip(client.display) < 0 || !client.seat || !client.notifier) {
        fprintf(stderr, "syauth-idle-lock: ext-idle-notify-v1 unavailable\n");
        if (client.seat) wl_seat_destroy(client.seat);
        wl_registry_destroy(client.registry);
        wl_display_disconnect(client.display);
        return 1;
    }

    client.notification = ext_idle_notifier_v1_get_idle_notification(
        client.notifier, client.timeout_ms, client.seat);
    ext_idle_notification_v1_add_listener(client.notification, &idle_listener, &client);
    if (ext_idle_notifier_v1_get_version(client.notifier) >= 2) {
        client.input_notification = ext_idle_notifier_v1_get_input_idle_notification(
            client.notifier, client.timeout_ms, client.seat);
        ext_idle_notification_v1_add_listener(client.input_notification, &input_idle_listener, &client);
        fprintf(stderr, "syauth-idle-lock: INPUT notification armed (v2, ignores inhibitors)\n");
    }
    if (wl_display_roundtrip(client.display) < 0) {
        fprintf(stderr, "syauth-idle-lock: Wayland event setup failed\n");
        return 1;
    }
    fprintf(stderr, "syauth-idle-lock: armed — will report idle after %u ms\n", client.timeout_ms);
    // ext-idle-notify-v1 notifications are reusable: one object emits an
    // idled/resumed pair for every idle cycle. Keep dispatching until the
    // Wayland connection itself fails.
    while (wl_display_dispatch(client.display) >= 0) {
    }
    int result = 1;
    if (client.input_notification) {
        ext_idle_notification_v1_destroy(client.input_notification);
    }
    ext_idle_notification_v1_destroy(client.notification);
    ext_idle_notifier_v1_destroy(client.notifier);
    wl_seat_destroy(client.seat);
    wl_registry_destroy(client.registry);
    wl_display_disconnect(client.display);
    return result;
}
