/* straws: the few POSIX <unistd.h> pieces nec2c uses, for Windows. */
#ifndef NEC_COMPAT_UNISTD_H
#define NEC_COMPAT_UNISTD_H

#include <time.h>

#define _SC_CLK_TCK 2
static inline long sysconf(int name) { (void)name; return CLOCKS_PER_SEC; }

extern char *optarg;
extern int optind, opterr, optopt;
int getopt(int argc, char *const argv[], const char *optstring);

#endif
