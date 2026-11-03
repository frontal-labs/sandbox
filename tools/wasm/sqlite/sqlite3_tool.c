/*
** `sqlite3`: a command-line SQLite for the sandbox, built as a WASI command.
**
**   sqlite3 [-header] [DATABASE] [SQL...]
**
** DATABASE is a path in the sandbox, or ":memory:" when omitted. Each SQL argument
** is run in order. With no SQL argument, the script is read from stdin. Results
** print in list mode: columns separated by '|', one row per line, NULL as an
** empty field, and a header line of column names with -header. The first error
** is reported as "Error: ..." on stderr, the rest of the script still runs, and the
** exit status is 1. The subset matches the SQLite CLI's list mode for these cases.
*/
#include "sqlite3.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
  int header;
  int printed_header;
} Output;

static int print_row(void *data, int argc, char **argv, char **names) {
  Output *out = (Output *)data;
  if (out->header && !out->printed_header) {
    for (int i = 0; i < argc; i++) {
      fputs(names[i], stdout);
      fputc(i + 1 < argc ? '|' : '\n', stdout);
    }
    out->printed_header = 1;
  }
  for (int i = 0; i < argc; i++) {
    if (argv[i]) fputs(argv[i], stdout);
    fputc(i + 1 < argc ? '|' : '\n', stdout);
  }
  return 0;
}

/* Runs one script. Returns 0 on success, 1 on an SQL error. */
static int run_script(sqlite3 *db, const char *sql, Output *out) {
  char *err = NULL;
  out->printed_header = 0;
  int rc = sqlite3_exec(db, sql, print_row, out, &err);
  if (rc != SQLITE_OK) {
    fprintf(stderr, "Error: %s\n", err ? err : sqlite3_errstr(rc));
    sqlite3_free(err);
    return 1;
  }
  return 0;
}

/* Reads all of stdin into a NUL-terminated buffer. */
static char *read_stdin(void) {
  size_t cap = 4096, len = 0;
  char *buf = malloc(cap);
  if (!buf) return NULL;
  size_t n;
  while ((n = fread(buf + len, 1, cap - len - 1, stdin)) > 0) {
    len += n;
    if (len + 1 >= cap) {
      cap *= 2;
      char *grown = realloc(buf, cap);
      if (!grown) { free(buf); return NULL; }
      buf = grown;
    }
  }
  buf[len] = '\0';
  return buf;
}

int main(int argc, char **argv) {
  Output out = {0, 0};
  int i = 1;
  if (i < argc && strcmp(argv[i], "-header") == 0) {
    out.header = 1;
    i++;
  }
  const char *path = ":memory:";
  if (i < argc) path = argv[i++];

  sqlite3 *db = NULL;
  if (sqlite3_open(path, &db) != SQLITE_OK) {
    fprintf(stderr, "Error: unable to open database \"%s\": %s\n", path,
            db ? sqlite3_errmsg(db) : "out of memory");
    sqlite3_close(db);
    return 1;
  }

  int status = 0;
  if (i < argc) {
    for (; i < argc; i++) status |= run_script(db, argv[i], &out);
  } else {
    char *script = read_stdin();
    if (!script) {
      fprintf(stderr, "Error: out of memory reading stdin\n");
      sqlite3_close(db);
      return 1;
    }
    status = run_script(db, script, &out);
    free(script);
  }

  if (sqlite3_close(db) != SQLITE_OK) {
    fprintf(stderr, "Error: %s\n", "database did not close cleanly");
    return 1;
  }
  return status;
}
