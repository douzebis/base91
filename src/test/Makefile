# SPDX-FileCopyrightText: 2025 2006 Joachim Henke
#
# SPDX-License-Identifier: BSD-3-Clause

SHELL = /bin/sh

.PHONY: all clean

all:
	ln -sf ../base91 b91enc
	ln -sf ../base91 b91dec
	$(SHELL) test.sh

clean:
	-rm -f b91??c *.b91 *.dat core
