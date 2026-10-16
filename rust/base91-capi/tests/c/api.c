/*
 * SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
 *
 * SPDX-License-Identifier: MIT
 */

/*
 * The C side of libbase91's tests (spec 0014 §4, passthru.tests.c-api):
 * built against the installed header with -Wall -Werror, it calls every
 * function, so a function missing from the library, renamed, or declared
 * with the wrong arguments fails here.
 */

#include <base91.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures = 0;

#define CHECK(cond)                                                       \
	do {                                                              \
		if (!(cond)) {                                            \
			fprintf(stderr, "%s:%d: CHECK(%s) failed\n",      \
			        __FILE__, __LINE__, #cond);               \
			failures++;                                       \
		}                                                         \
	} while (0)

static unsigned char data[10000];

static void henke_api(void)
{
	struct basE91 b;
	unsigned char enc[20000], dec[10000];
	size_t n = 0, m = 0, i;

	/* Henke's layout on LP64. */
	CHECK(sizeof(struct basE91) == 2 * sizeof(unsigned long));

	basE91_init(&b);
	CHECK(b.queue == 0 && b.nbits == 0 && b.val == -1);
	for (i = 0; i < sizeof data; i += 777) {
		size_t len = sizeof data - i < 777 ? sizeof data - i : 777;
		n += basE91_encode(&b, data + i, len, enc + n);
	}
	n += basE91_encode_end(&b, enc + n);

	basE91_init(&b);
	for (i = 0; i < n; i += 333) {
		size_t len = n - i < 333 ? n - i : 333;
		m += basE91_decode(&b, enc + i, len, dec + m);
	}
	m += basE91_decode_end(&b, dec + m);
	CHECK(m == sizeof data);
	CHECK(memcmp(dec, data, sizeof data) == 0);
}

static void new_api(void)
{
	static unsigned char enc[20000], dec[20000];
	size_t written = 0, offset = (size_t)-1, need;
	int format;

	for (format = BASE91_HENKE; format <= BASE91_SIMD; format++) {
		size_t wrap = format == BASE91_SIMD ? 64 : 0;

		need = base91_encoded_len_max((base91_format)format, sizeof data, wrap);
		CHECK(need > 0 && need <= sizeof enc);
		CHECK(base91_encode((base91_format)format, data, sizeof data, wrap,
		                    enc, need, &written) == BASE91_OK);
		CHECK(written <= need);
		CHECK((format == BASE91_SIMD) == (enc[0] == '-'));

		need = base91_decoded_len_max(written);
		CHECK(base91_decode(enc, written, dec, need, &written, &offset) == BASE91_OK);
		CHECK(written == sizeof data);
		CHECK(memcmp(dec, data, sizeof data) == 0);
	}

	/* The errors. */
	CHECK(base91_encode(BASE91_SIMD, data, 10, 5, enc, sizeof enc, &written)
	      == BASE91_ERR_INVALID_WIDTH);
	CHECK(base91_encode(BASE91_SIMD, data, 100, 0, enc, 10, &written)
	      == BASE91_ERR_BUFFER_TOO_SMALL);
	CHECK(written == base91_encoded_len_max(BASE91_SIMD, 100, 0));
	CHECK(base91_encode(BASE91_HENKE, NULL, 10, 0, enc, sizeof enc, &written)
	      == BASE91_ERR_NULL_POINTER);
	CHECK(base91_encode(BASE91_SIMD, data, 1000, 0, enc, sizeof enc, &written)
	      == BASE91_OK);
	enc[10] = '\'';
	CHECK(base91_decode(enc, written, dec, sizeof dec, &written, &offset)
	      == BASE91_ERR_INVALID_CHARACTER);
	CHECK(offset == 10);
	CHECK(strcmp(base91_strerror(BASE91_ERR_INVALID_CHARACTER), "invalid character") == 0);
}

static void strings(void)
{
	const char *level = base91_simd_level();

	CHECK(strcmp(level, "avx2") == 0 || strcmp(level, "sse4.1") == 0 ||
	      strcmp(level, "neon") == 0 || strcmp(level, "scalar") == 0);
	/* The best kernel of the CPU, detected at run time: a library built
	 * for a generic target must not fall back to the scalar kernel. */
#if defined(__x86_64__)
	__builtin_cpu_init();
	if (__builtin_cpu_supports("avx2"))
		CHECK(strcmp(level, "avx2") == 0);
	else if (__builtin_cpu_supports("sse4.1"))
		CHECK(strcmp(level, "sse4.1") == 0);
#elif defined(__aarch64__)
	CHECK(strcmp(level, "neon") == 0);
#endif
	CHECK(strlen(base91_version()) > 0);
	printf("libbase91 %s, SIMD level %s\n", base91_version(), level);
}

int main(void)
{
	size_t i;
	unsigned long x = 12345;

	for (i = 0; i < sizeof data; i++) {
		x = x * 6364136223846793005UL + 1442695040888963407UL;
		data[i] = (unsigned char)(x >> 56);
	}
	henke_api();
	new_api();
	strings();
	if (failures) {
		fprintf(stderr, "%d failure(s)\n", failures);
		return 1;
	}
	return 0;
}
