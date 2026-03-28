/*
 * SPDX-FileCopyrightText: 2000-2006 Joachim Henke
 *
 * SPDX-License-Identifier: BSD-3-Clause
 */

#ifndef BASE91_H
#define BASE91_H 1

#include <stddef.h>

struct basE91 {
	unsigned long queue;
	unsigned int nbits;
	int val;
};

void basE91_init(struct basE91 *b);

size_t basE91_encode(struct basE91 * __restrict__ b,
                     const void * __restrict__ i, size_t len,
                     void * __restrict__ o);

size_t basE91_encode_end(struct basE91 *b, void *o);

size_t basE91_decode(struct basE91 * __restrict__ b,
                     const void * __restrict__ i, size_t len,
                     void * __restrict__ o);

size_t basE91_decode_end(struct basE91 *b, void *o);

#endif	/* base91.h */
