#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <time.h>

/* Freeze only wall time; daemon deadlines and Tokio timers remain monotonic. */
static int (*real_clock_gettime)(clockid_t, struct timespec *);

__attribute__((constructor)) static void resolve_clock(void) {
    /* SAFETY: POSIX dlsym resolves the matching function signature; initialization
       finishes before the daemon starts worker threads. */
    real_clock_gettime = (int (*)(clockid_t, struct timespec *))
        dlsym(RTLD_NEXT, "clock_gettime");
}

int clock_gettime(clockid_t clock, struct timespec *value) {
    if (clock == CLOCK_REALTIME) {
        value->tv_sec = 2000000000;
        value->tv_nsec = 0;
        return 0;
    }
    if (!real_clock_gettime) {
        errno = ENOSYS;
        return -1;
    }
    return real_clock_gettime(clock, value);
}
