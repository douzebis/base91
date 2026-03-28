/*
 * SPDX-FileCopyrightText: 2000-2006 Joachim Henke
 * SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
 *
 * SPDX-License-Identifier: BSD-3-Clause
 */

#include "base91.h"

static const unsigned char enctab[91] = {
	'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
	'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
	'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
	'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
	'0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '!', '#', '$',
	'%', '&', '(', ')', '*', '+', ',', '.', '/', ':', ';', '<', '=',
	'>', '?', '@', '[', ']', '^', '_', '`', '{', '|', '}', '~', '"'
};
static const unsigned char dectab[256] = {
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 62, 90, 63, 64, 65, 66, 91, 67, 68, 69, 70, 71, 91, 72, 73,
	52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 74, 75, 76, 77, 78, 79,
	80,  0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14,
	15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 81, 91, 82, 83, 84,
	85, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
	41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 86, 87, 88, 89, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91
};

void basE91_init(struct basE91 *b)
{
	b->queue = 0;
	b->nbits = 0;
	b->val = -1;
}

size_t basE91_encode(struct basE91 * __restrict__ b,
                     const void * __restrict__ i, size_t len,
                     void * __restrict__ o)
{
	const unsigned char *ib = i;
	unsigned char *ob = o;
	size_t n = 0;
	unsigned long queue = b->queue;
	unsigned int nbits = b->nbits;

	while (len--) {
		queue |= (unsigned long)*ib++ << nbits;
		nbits += 8;
		if (nbits > 13) {	/* enough bits in queue */
			unsigned int val = queue & 8191;

			/* Duplicate writes per arm so Clang keeps each path
			 * separate and uses immediate-count shifts instead of
			 * merging both arms into a variable-count shr cl. */
			if (val > 88) {
				queue >>= 13;
				nbits -= 13;
				ob[n] = enctab[val % 91];
				ob[n + 1] = enctab[val / 91];
			} else {	/* we can take 14 bits */
				val = queue & 16383;
				queue >>= 14;
				nbits -= 14;
				ob[n] = enctab[val % 91];
				ob[n + 1] = enctab[val / 91];
			}
			n += 2;
		}
	}

	b->queue = queue;
	b->nbits = nbits;
	return n;
}

/* process remaining bits from bit queue; write up to 2 bytes */

size_t basE91_encode_end(struct basE91 *b, void *o)
{
	unsigned char *ob = o;
	size_t n = 0;

	if (b->nbits) {
		ob[n++] = enctab[b->queue % 91];
		if (b->nbits > 7 || b->queue > 90)
			ob[n++] = enctab[b->queue / 91];
	}
	b->queue = 0;
	b->nbits = 0;
	b->val = -1;

	return n;
}

size_t basE91_decode(struct basE91 * __restrict__ b,
                     const void * __restrict__ i, size_t len,
                     void * __restrict__ o)
{
	const unsigned char *ib = i;
	const unsigned char *ie = ib + len;
	unsigned char *ob = o;
	size_t n = 0;
	unsigned long queue = b->queue;
	unsigned int nbits = b->nbits;
	unsigned int d0, d1;

	/* Resume a pending first char from a previous call. */
	if (b->val != -1) {
		d0 = (unsigned int)b->val;
		b->val = -1;
		goto have_d0;
	}

	for (;;) {
		/* --- scan for d0 --- */
		do {
			if (ib == ie) goto done;
			d0 = dectab[*ib++];
		} while (d0 == 91);

have_d0:
		/* --- scan for d1 --- */
		do {
			if (ib == ie) { b->val = (int)d0; goto done; }
			d1 = dectab[*ib++];
		} while (d1 == 91);

		/* --- emit --- */
		{
			unsigned int v = d0 + d1 * 91;
			queue |= (unsigned long)v << nbits;
			nbits += (v & 8191) > 88 ? 13 : 14;
			ob[n++] = (unsigned char)queue;
			queue >>= 8;
			nbits -= 8;
			if (nbits >= 8) {
				ob[n++] = (unsigned char)queue;
				queue >>= 8;
				nbits -= 8;
			}
		}
	}

done:
	b->queue = queue;
	b->nbits = nbits;
	return n;
}

/* process remaining bits; write at most 1 byte */

size_t basE91_decode_end(struct basE91 *b, void *o)
{
	unsigned char *ob = o;
	size_t n = 0;

	if (b->val != -1)
		ob[n++] = b->queue | b->val << b->nbits;
	b->queue = 0;
	b->nbits = 0;
	b->val = -1;

	return n;
}
