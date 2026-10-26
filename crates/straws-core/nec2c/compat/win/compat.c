/* straws: Windows support code for nec2c (see compat/win/*.h). */
#include <string.h>
#include <stdio.h>
#include <math.h>

/* ---- getopt(): just enough for nec2c's "i:o:hv" options ---- */
char *optarg = NULL;
int optind = 1, opterr = 1, optopt = 0;

int getopt(int argc, char *const argv[], const char *optstring)
{
  static int pos = 1;
  const char *spec;
  char *arg;

  if (optind >= argc)
    return -1;
  arg = argv[optind];
  if (arg[0] != '-' || arg[1] == '\0')
    return -1;
  if (strcmp(arg, "--") == 0) {
    optind++;
    return -1;
  }

  optopt = arg[pos];
  spec = strchr(optstring, optopt);
  if (spec == NULL || optopt == ':') {
    if (arg[++pos] == '\0') {
      optind++;
      pos = 1;
    }
    return '?';
  }

  if (spec[1] == ':') {
    if (arg[pos + 1] != '\0') {
      optarg = &arg[pos + 1];
    } else if (optind + 1 < argc) {
      optarg = argv[++optind];
    } else {
      optind++;
      pos = 1;
      return '?';
    }
    optind++;
    pos = 1;
  } else if (arg[++pos] == '\0') {
    optind++;
    pos = 1;
  }
  return optopt;
}

/* ---- Complex multiply and divide helpers ----
 * clang emits calls to these for `_Complex double` arithmetic. On Linux and
 * macOS they come from the compiler runtime; the MSVC toolchain does not
 * link one, so they are provided here. Multiplication is the textbook
 * formula; division uses Smith's method (as libgcc does) to avoid overflow. */

double _Complex __muldc3(double a, double b, double c, double d)
{
  double _Complex z;
  __real__ z = a * c - b * d;
  __imag__ z = a * d + b * c;
  return z;
}

double _Complex __divdc3(double a, double b, double c, double d)
{
  double _Complex z;
  double ratio, denom;
  if (fabs(c) < fabs(d)) {
    ratio = c / d;
    denom = c * ratio + d;
    __real__ z = (a * ratio + b) / denom;
    __imag__ z = (b * ratio - a) / denom;
  } else {
    ratio = d / c;
    denom = d * ratio + c;
    __real__ z = (b * ratio + a) / denom;
    __imag__ z = (b - a * ratio) / denom;
  }
  return z;
}
