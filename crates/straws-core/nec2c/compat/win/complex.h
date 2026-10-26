/* straws: C99 <complex.h> for clang-cl on Windows.
 *
 * Microsoft's C runtime has no usable C99 complex arithmetic, but clang
 * supports the `_Complex` type natively. This header replaces the system
 * <complex.h> (it is found first on the include path) and supplies the
 * handful of complex functions nec2c uses, written with real maths only.
 * Only used on Windows; Linux and macOS use their own <complex.h>. */
#ifndef NEC_COMPAT_COMPLEX_H
#define NEC_COMPAT_COMPLEX_H

#include <math.h>

#define complex _Complex
#define _Complex_I (__extension__ 1.0iF)
#define I _Complex_I

static inline double creal(double _Complex z) { return __builtin_creal(z); }
static inline double cimag(double _Complex z) { return __builtin_cimag(z); }
static inline double _Complex conj(double _Complex z) { return __builtin_conj(z); }
static inline double cabs(double _Complex z) { return hypot(creal(z), cimag(z)); }
static inline double carg(double _Complex z) { return atan2(cimag(z), creal(z)); }

static inline double _Complex nec_cmplx(double re, double im)
{
  double _Complex z;
  __real__ z = re;
  __imag__ z = im;
  return z;
}

static inline double _Complex cexp(double _Complex z)
{
  double r = exp(creal(z));
  return nec_cmplx(r * cos(cimag(z)), r * sin(cimag(z)));
}

static inline double _Complex clog(double _Complex z)
{
  return nec_cmplx(log(cabs(z)), carg(z));
}

/* Principal square root, same branch cut as C99 csqrt(). */
static inline double _Complex csqrt(double _Complex z)
{
  double re = creal(z), im = cimag(z), t;
  if (re == 0.0 && im == 0.0)
    return nec_cmplx(0.0, im);
  t = sqrt((fabs(re) + cabs(z)) * 0.5);
  if (re >= 0.0)
    return nec_cmplx(t, im / (2.0 * t));
  return nec_cmplx(fabs(im) / (2.0 * t), copysign(t, im));
}

#endif
