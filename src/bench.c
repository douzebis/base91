/*
 * SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
 *
 * SPDX-License-Identifier: MIT
 */

/*
 * Standalone throughput benchmark for the C reference basE91 implementation.
 * Measures encode and decode throughput on 1 MiB of random data.
 * Prints results in MiB/s to stdout.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "base91.h"

#define SIZE (1024 * 1024)
#define WARMUP_ITERS 5
#define BENCH_ITERS  50

static double now_sec(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec + ts.tv_nsec * 1e-9;
}

int main(void)
{
	/* ---- allocate buffers ---- */
	unsigned char *input  = malloc(SIZE);
	/* encode output: upper bound = SIZE * 16/13 + 2 */
	size_t enc_cap = (size_t)SIZE * 16 / 13 + 4;
	unsigned char *enc_buf = malloc(enc_cap);
	unsigned char *dec_buf = malloc(SIZE + 2);

	if (!input || !enc_buf || !dec_buf) { fputs("OOM\n", stderr); return 1; }

	/* fill with pseudo-random bytes */
	srand(42);
	for (size_t i = 0; i < SIZE; i++)
		input[i] = (unsigned char)(rand() & 0xff);

	/* ---- pre-encode for the decode benchmark ---- */
	struct basE91 state;
	basE91_init(&state);
	size_t enc_len = basE91_encode(&state, input, SIZE, enc_buf);
	enc_len += basE91_encode_end(&state, enc_buf + enc_len);

	/* ================================================================
	 * ENCODE benchmark
	 * ================================================================ */
	size_t enc_n = 0;

	/* warmup */
	for (int i = 0; i < WARMUP_ITERS; i++) {
		basE91_init(&state);
		enc_n  = basE91_encode(&state, input, SIZE, enc_buf);
		enc_n += basE91_encode_end(&state, enc_buf + enc_n);
	}

	double t0 = now_sec();
	for (int i = 0; i < BENCH_ITERS; i++) {
		basE91_init(&state);
		enc_n  = basE91_encode(&state, input, SIZE, enc_buf);
		enc_n += basE91_encode_end(&state, enc_buf + enc_n);
	}
	double t1 = now_sec();

	double enc_mib_s = (double)SIZE * BENCH_ITERS / (t1 - t0) / (1024.0 * 1024.0);
	printf("C encode: %.1f MiB/s  (input %zu bytes → %zu encoded bytes)\n",
	       enc_mib_s, (size_t)SIZE, enc_n);

	/* ================================================================
	 * DECODE benchmark
	 * ================================================================ */
	size_t dec_n = 0;

	/* warmup */
	for (int i = 0; i < WARMUP_ITERS; i++) {
		basE91_init(&state);
		dec_n  = basE91_decode(&state, enc_buf, enc_len, dec_buf);
		dec_n += basE91_decode_end(&state, dec_buf + dec_n);
	}

	t0 = now_sec();
	for (int i = 0; i < BENCH_ITERS; i++) {
		basE91_init(&state);
		dec_n  = basE91_decode(&state, enc_buf, enc_len, dec_buf);
		dec_n += basE91_decode_end(&state, dec_buf + dec_n);
	}
	t1 = now_sec();

	double dec_mib_s = (double)enc_len * BENCH_ITERS / (t1 - t0) / (1024.0 * 1024.0);
	printf("C decode: %.1f MiB/s  (input %zu encoded bytes → %zu decoded bytes)\n",
	       dec_mib_s, enc_len, dec_n);

	free(input); free(enc_buf); free(dec_buf);
	return 0;
}
