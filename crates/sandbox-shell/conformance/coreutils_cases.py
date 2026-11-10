"""Golden cases for the coreutils builtins (PLAN.md §4.4, §9 M2).

Each script runs unchanged under the GNU coreutils and the sandbox shell, and writes
only what it needs to compare. `record_coreutils.py` runs every case with the GNU
tools first on PATH and writes `coreutils.json`, which the Rust test replays. Expected
output is recorded from GNU, never written by hand. Scripts use relative paths only.
"""

CASES = []


def case(name, script):
    CASES.append((name, script))


# --- sort -------------------------------------------------------------------
case("sort_plain", "printf 'pear\\napple\\nfig\\n' > f; sort f")
case("sort_reverse", "printf 'pear\\napple\\nfig\\n' > f; sort -r f")
case("sort_numeric", "printf '10\\n9\\n-3\\n2.5\\nx\\n' > f; sort -n f")
case("sort_numeric_reverse", "printf '10\\n9\\n-3\\n2.5\\n' > f; sort -nr f")
case("sort_unique", "printf 'b\\na\\nb\\na\\n' > f; sort -u f")
case("sort_numeric_unique", "printf '1\\n01\\n2\\n' > f; sort -nu f")
case("sort_stdin", "printf 'c\\na\\nb\\n' | sort")
case("sort_two_files", "printf 'b\\n' > a; printf 'a\\n' > b; sort a b")
case("sort_empty", "printf '' > f; sort f; echo done")
case("sort_no_trailing_newline", "printf 'b\\na' > f; sort f")
case("sort_missing_file", "sort nope.txt; echo \"status $?\"")

# --- uniq -------------------------------------------------------------------
case("uniq_plain", "printf 'a\\na\\nb\\nb\\nb\\na\\n' > f; uniq f")
case("uniq_count", "printf 'a\\na\\nb\\n' > f; uniq -c f")
case("uniq_repeated_only", "printf 'a\\na\\nb\\nc\\nc\\n' > f; uniq -d f")
case("uniq_unique_only", "printf 'a\\na\\nb\\nc\\nc\\n' > f; uniq -u f")
case("uniq_stdin", "printf 'x\\nx\\ny\\n' | uniq")
case("uniq_nonadjacent_kept", "printf 'a\\nb\\na\\n' > f; uniq f")

# --- cut --------------------------------------------------------------------
case("cut_field_one", "printf 'a:b:c\\nd:e:f\\n' > f; cut -d: -f1 f")
case("cut_field_range", "printf 'a:b:c:d\\n' > f; cut -d: -f2-3 f")
case("cut_field_open_start", "printf 'a:b:c:d\\n' > f; cut -d: -f-2 f")
case("cut_field_open_end", "printf 'a:b:c:d\\n' > f; cut -d: -f3- f")
case("cut_field_list", "printf 'a:b:c:d\\n' > f; cut -d: -f1,3 f")
case("cut_tab_default", "printf 'a\\tb\\tc\\n' > f; cut -f2 f")
case("cut_suppress_undelimited", "printf 'abc\\nx:y\\n' > f; cut -s -d: -f2 f")
case("cut_passes_undelimited", "printf 'abc\\nx:y\\n' > f; cut -d: -f2 f")
case("cut_chars", "printf 'abcdef\\n' > f; cut -c2-4 f")
case("cut_chars_list", "printf 'abcdef\\n' > f; cut -c1,3,5 f")
case("cut_no_list", "printf 'a\\n' > f; cut f; echo \"status $?\"")

# --- tr ---------------------------------------------------------------------
case("tr_translate", "printf 'hello\\n' | tr a-z A-Z")
case("tr_delete", "printf 'a1b2c3\\n' | tr -d 0-9")
case("tr_squeeze_set1", "printf 'aaabbbccc\\n' | tr -s a-c")
case("tr_newline_to_space", "printf 'one\\ntwo\\n' | tr '\\n' ' '")
case("tr_short_set2", "printf 'abcd\\n' | tr abcd xy")
case("tr_translate_and_squeeze", "printf 'aabbcc\\n' | tr -s ab xy")
case("tr_escape_tab", "printf 'a\\tb\\n' | tr '\\t' '_'")

# --- tee --------------------------------------------------------------------
case("tee_writes_file_and_stdout", "printf 'hi\\n' | tee out; cat out")
case("tee_append", "printf 'one\\n' > out; printf 'two\\n' | tee -a out; cat out")
case("tee_overwrite", "printf 'one\\n' > out; printf 'two\\n' | tee out; cat out")

# --- seq --------------------------------------------------------------------
case("seq_last_only", "seq 3")
case("seq_first_last", "seq 2 5")
case("seq_increment", "seq 1 2 7")
case("seq_descending", "seq 5 -2 1")
case("seq_separator", "seq -s, 3")
case("seq_equal_width", "seq -w 8 10")
case("seq_empty_range", "seq 5 1; echo done")
case("seq_zero_increment", "seq 1 0 3; echo \"status $?\"")

# --- paste ------------------------------------------------------------------
case("paste_two_files", "printf 'a\\nb\\n' > x; printf '1\\n2\\n' > y; paste x y")
case("paste_delimiter", "printf 'a\\nb\\n' > x; printf '1\\n2\\n' > y; paste -d, x y")
case("paste_unequal", "printf 'a\\nb\\nc\\n' > x; printf '1\\n' > y; paste x y")
case("paste_serial", "printf 'a\\nb\\nc\\n' > x; paste -s x")
case("paste_serial_delimiter", "printf 'a\\nb\\nc\\n' > x; paste -s -d: x")

# --- rev --------------------------------------------------------------------
case("rev_lines", "printf 'abc\\nxy\\n' | rev")
case("rev_empty_line", "printf 'a\\n\\nb\\n' | rev")

# --- basename / dirname -----------------------------------------------------
case("basename_plain", "basename a/b/c.txt")
case("basename_suffix", "basename a/b/c.txt .txt")
case("basename_s_option", "basename -s .txt a/b/c.txt")
case("basename_trailing_slash", "basename a/b/")
case("basename_root", "basename /")
case("basename_multiple", "basename -a x/a y/b")
case("dirname_nested", "dirname a/b/c")
case("dirname_bare", "dirname file")
case("dirname_absolute", "dirname /usr")
case("dirname_double_slash", "dirname a//b//")
case("dirname_root", "dirname /")
case("dirname_multiple", "dirname a/b x/y/z")

# --- printenv ---------------------------------------------------------------
case("printenv_named", "export GREETING=hello; printenv GREETING")
case("printenv_missing", "printenv NOT_SET_ANYWHERE; echo \"status $?\"")

# --- cmp --------------------------------------------------------------------
case("cmp_equal", "printf 'abc\\n' > a; printf 'abc\\n' > b; cmp a b; echo \"status $?\"")
case("cmp_differ", "printf 'abc\\n' > a; printf 'abd\\n' > b; cmp a b; echo \"status $?\"")
case("cmp_differ_line_two", "printf 'x\\ny\\n' > a; printf 'x\\nz\\n' > b; cmp a b; echo \"status $?\"")
case("cmp_silent", "printf 'abc\\n' > a; printf 'abd\\n' > b; cmp -s a b; echo \"status $?\"")
case("cmp_eof", "printf 'abc\\n' > a; printf 'ab' > b; cmp a b; echo \"status $?\"")

# --- sed (substitute command) ------------------------------------------------
case("sed_first_match", "printf 'aaa\\n' | sed 's/a/b/'")
case("sed_global", "printf 'aaa\\n' | sed 's/a/b/g'")
case("sed_bre_groups", "printf 'ab12\\n' | sed 's/\\([a-z]*\\)\\([0-9]*\\)/[\\2\\1]/'")
case("sed_ere_groups", "printf 'hello world\\n' | sed -E 's/(\\w+) (\\w+)/\\2 \\1/'")
case("sed_ampersand", "printf 'cat\\n' | sed 's/cat/[&]/'")
case("sed_other_delimiter", "printf '/usr/bin\\n' | sed 's|/usr|/opt|'")
case("sed_no_match", "printf 'x\\n' | sed 's/y/z/'")
case("sed_each_line", "printf 'a1\\na2\\n' | sed 's/[0-9]/#/g'")
case("sed_ere_plus", "printf 'aaa b\\n' | sed -E 's/a+/X/'")
case("sed_bre_plus_is_literal", "printf 'a+b\\n' | sed 's/a+b/X/'")
case("sed_ignore_case", "printf 'Hello\\n' | sed 's/hello/X/i'")
case("sed_quiet_with_p", "printf 'a\\nb\\n' | sed -n 's/a/X/p'")
case("sed_from_file", "printf 'foo\\n' > f; sed 's/foo/bar/' f")

# --- diff (normal format) -----------------------------------------------------
case("diff_same", "printf 'a\\n' > x; printf 'a\\n' > y; diff x y; echo \"status $?\"")
case("diff_change", "printf 'a\\nb\\nc\\n' > x; printf 'a\\nB\\nc\\n' > y; diff x y; echo \"status $?\"")
case("diff_add", "printf 'a\\nc\\n' > x; printf 'a\\nb\\nc\\n' > y; diff x y; echo \"status $?\"")
case("diff_delete", "printf 'a\\nb\\nc\\n' > x; printf 'a\\nc\\n' > y; diff x y; echo \"status $?\"")
case("diff_multi_line_change", "printf 'a\\nb\\nc\\nd\\n' > x; printf 'a\\nX\\nY\\nd\\n' > y; diff x y; echo \"status $?\"")
case("diff_from_empty", "printf '' > x; printf 'z\\n' > y; diff x y; echo \"status $?\"")

# --- nl -----------------------------------------------------------------------
case("nl_default_skips_empty", "printf 'a\\n\\nb\\n' | nl")
case("nl_number_all", "printf 'a\\n\\nb\\n' | nl -b a")
case("nl_from_file", "printf 'x\\ny\\n' > f; nl f")

# --- base64 -------------------------------------------------------------------
case("base64_encode", "printf 'hello' | base64")
case("base64_wrap", "printf 'abcdefghijklmnopqrstuvwxyz0123456789' | base64 -w 10")
case("base64_no_wrap", "printf 'abcdefgh' | base64 -w 0")
case("base64_decode", "printf 'aGVsbG8=\\n' | base64 -d")
case("base64_empty", "printf '' | base64; echo \"status $?\"")

# --- date (UTC, explicit instants only) ---------------------------------------
case("date_epoch_day", "date -u -d @0 +%Y-%m-%d")
case("date_instant", "date -u -d @1700000000 '+%Y-%m-%d %H:%M:%S'")
case("date_weekday", "date -u -d @0 +%a")
case("date_day_of_year", "date -u -d @86400 +%j")

# --- find -----------------------------------------------------------------------
case("find_all", "mkdir -p d/sub; printf x > d/a.txt; printf y > d/sub/b.txt; printf z > d/c.md; find d | sort")
case("find_name_glob", "mkdir -p d/sub; printf x > d/a.txt; printf y > d/sub/b.txt; printf z > d/c.md; find d -name '*.txt' | sort")
case("find_type_f", "mkdir -p d/sub; printf x > d/a.txt; printf y > d/sub/b.txt; find d -type f | sort")
case("find_type_d", "mkdir -p d/sub; printf x > d/a.txt; find d -type d | sort")
case("find_maxdepth", "mkdir -p d/sub; printf x > d/a.txt; printf y > d/sub/b.txt; find d -maxdepth 1 | sort")
case("find_missing_start", "find nope; echo \"status $?\"")

# --- xargs ----------------------------------------------------------------------
case("xargs_default_echo", "printf 'x y\\n' | xargs")
case("xargs_echo_lines", "printf 'a b\\nc\\n' | xargs echo")
case("xargs_one_per_run", "printf 'a b c\\n' | xargs -n 1 echo")
case("xargs_fixed_prefix", "printf 'b\\n' | xargs echo a")
case("xargs_quoted_word", "printf \"'x y' z\\n\" | xargs echo")
case("xargs_cat_file", "printf 'hello\\n' > f; echo f | xargs cat")
case("xargs_empty_input", "printf '' | xargs echo done")

# --- du -------------------------------------------------------------------------
case("du_file_bytes", "printf '12345' > f; du -sb f")
case("du_two_files", "printf 'ab' > x; printf 'cde' > y; du -sb x y")

# --- stat -----------------------------------------------------------------------
case("stat_size_and_name", "printf 'abcd' > f; stat -c '%s %n' f")
case("stat_regular_file", "printf 'abcd' > f; stat -c %F f")
case("stat_directory", "mkdir d; stat -c %F d")

# --- realpath (relative output only, so the host directory never shows) ----------
case("realpath_relative", "mkdir -p a/b; realpath --relative-to=a a/b")
case("realpath_parent", "mkdir -p a/b; realpath --relative-to=a/b a")
case("realpath_missing_last", "realpath nope >/dev/null; echo \"status $?\"")
case("realpath_missing_parent", "realpath nope/x >/dev/null; echo \"status $?\"")

# --- commands that predate the coreutils module --------------------------------
case("cat_files", "printf 'a\\n' > x; printf 'b\\n' > y; cat x y")
case("cat_stdin_dash", "printf 'in\\n' | cat")
case("cat_missing_status", "cat nope; echo \"status $?\"")
case("cp_file", "printf 'data\\n' > a; cp a b; cat b")
case("cp_into_dir", "printf 'data\\n' > a; mkdir d; cp a d; cat d/a")
case("mv_file", "printf 'data\\n' > a; mv a b; cat b; echo \"status $?\"")
case("rm_file", "printf 'x\\n' > a; rm a; echo \"status $?\"")
case("rm_missing", "rm nope; echo \"status $?\"")
case("mkdir_parents", "mkdir -p a/b/c; find a | sort")
case("mkdir_exists", "mkdir d; mkdir d; echo \"status $?\"")
case("touch_creates", "touch new; echo \"status $?\"")
case("head_default", "printf '1\\n2\\n3\\n4\\n5\\n6\\n7\\n8\\n9\\n10\\n11\\n' > f; head f")
case("head_n", "printf 'a\\nb\\nc\\n' > f; head -n 2 f")
case("tail_n", "printf 'a\\nb\\nc\\n' > f; tail -n 2 f")
case("wc_lines", "printf 'a\\nb\\n' | wc -l")
case("wc_words_chars", "printf 'one two\\nthree\\n' | wc -w")
case("grep_match", "printf 'apple\\nbanana\\n' | grep an")
case("grep_no_match_status", "printf 'apple\\n' | grep zzz; echo \"status $?\"")
case("grep_count_style", "printf 'x\\ny\\nx\\n' | grep -c x")
