/*
 * A reactor entry for CPython, so the interpreter can be initialised once and snapshotted with
 * Wizer (docs/PYTHON_WASM.md). The command entry (`_start`) initialises the interpreter on every
 * run; this one does it once, in sandbox_init, and sandbox_run executes a script in that
 * interpreter.
 *
 * The host writes a script into the buffer from sandbox_buffer, then calls sandbox_run with its
 * length. The script runs as `python -c <script>` would: sys.argv is ['-c'], and the script's
 * exit status is the process's. A script that raises exits 1, as native CPython does.
 */
#include <Python.h>
#include <stdio.h>
#include <string.h>

#define SCRIPT_CAPACITY (1024 * 1024)

static char script[SCRIPT_CAPACITY + 1];

/* The script buffer. The host writes at most SCRIPT_CAPACITY bytes into it. */
__attribute__((export_name("sandbox_buffer")))
char *sandbox_buffer(void) {
    return script;
}

/* Initialises the interpreter with the standard library at /usr/lib, as a zip. A failed start
 * traps, which stops the pre-initialisation. */
__attribute__((export_name("sandbox_init")))
void sandbox_init(void) {
    PyConfig config;
    PyConfig_InitPythonConfig(&config);
    config.isolated = 0;
    config.site_import = 1;
    config.install_signal_handlers = 0;
    /* A fixed hash seed. Wizer runs this with the platform's randomness, so a random seed would
     * make the snapshot differ on every build. The host's random_get is a fixed pattern anyway. */
    config.use_hash_seed = 1;
    config.hash_seed = 0;
    config.home = L"/usr";
    config.module_search_paths_set = 1;
    PyWideStringList_Append(&config.module_search_paths, L"/usr/lib/python313.zip");
    PyWideStringList_Append(&config.module_search_paths, L"/usr/lib/python3.13");
    PyStatus status = Py_InitializeFromConfig(&config);
    PyConfig_Clear(&config);
    if (PyStatus_IsError(status)) {
        __builtin_trap();
    }
    /* Modules that most scripts import are loaded now, so the snapshot holds them and a run does
     * not compile them from source again. A failure here traps, which fails the build. */
    if (PyRun_SimpleString("import collections, json, re") != 0) {
        __builtin_trap();
    }
}

/* Runs the first `length` bytes of the script buffer. Returns 0 when the script completes,
 * and 1 when it raises. A script that calls sys.exit exits with its status, from inside. */
__attribute__((export_name("sandbox_run")))
int sandbox_run(int length) {
    if (length < 0 || length > SCRIPT_CAPACITY) {
        return 1;
    }
    script[length] = '\0';

    wchar_t *argv[] = {L"-c"};
    PySys_SetArgvEx(1, argv, 1);

    int status = PyRun_SimpleString(script);

    /* Native CPython flushes at exit. This interpreter stays alive for the run, so flush here. */
    PyRun_SimpleString("import sys\ntry:\n sys.stdout.flush()\n sys.stderr.flush()\nexcept Exception:\n pass\n");
    return status == 0 ? 0 : 1;
}
