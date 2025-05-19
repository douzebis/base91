# SPDX-FileCopyrightText: 2025 2006 Joachim Henke
#
# SPDX-License-Identifier: BSD-3-Clause

CC = gcc
CFLAGS = -Wall -W -O2
LDFLAGS = -s
INSTALL = install
INSTALL_PROGRAM = $(INSTALL) -m 555
INSTALL_DATA = $(INSTALL) -m 444

prefix ?= /usr/local
bindir = $(prefix)/bin
mandir = $(prefix)/share/man/man1
manext = .1

BIN = base91
OBJS = cli.o base91.o

.PHONY: all install clean

all: $(BIN)

$(BIN): $(OBJS)
	$(CC) $(LDFLAGS) -o $@ $^

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

install: all
	mkdir -p $(DESTDIR)$(bindir)
	$(INSTALL_PROGRAM) $(BIN) $(DESTDIR)$(bindir)/$(BIN)
	ln -sf $(BIN) $(DESTDIR)$(bindir)/b91dec
	ln -sf $(BIN) $(DESTDIR)$(bindir)/b91enc

	mkdir -p $(DESTDIR)$(mandir)
	$(INSTALL_DATA) base91.1 $(DESTDIR)$(mandir)/base91$(manext)
	ln -sf base91$(manext) $(DESTDIR)$(mandir)/b91dec$(manext)
	ln -sf base91$(manext) $(DESTDIR)$(mandir)/b91enc$(manext)

clean:
	rm -f $(OBJS) $(BIN)
