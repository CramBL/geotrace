#ifndef GEOTRACE_C_TEST_HELPERS_H
#define GEOTRACE_C_TEST_HELPERS_H

#include <criterion/criterion.h>
#include <math.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

#endif /* GEOTRACE_C_TEST_HELPERS_H */
