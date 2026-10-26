/* straws: minimal POSIX times() for Windows, used for nec2c's run
 * timer. Reports processor time from clock(). */
#ifndef NEC_COMPAT_SYS_TIMES_H
#define NEC_COMPAT_SYS_TIMES_H

#include <time.h>

struct tms {
  clock_t tms_utime;
  clock_t tms_stime;
  clock_t tms_cutime;
  clock_t tms_cstime;
};

static inline clock_t times(struct tms *buf)
{
  clock_t c = clock();
  buf->tms_utime = c;
  buf->tms_stime = 0;
  buf->tms_cutime = 0;
  buf->tms_cstime = 0;
  return c;
}

#endif
