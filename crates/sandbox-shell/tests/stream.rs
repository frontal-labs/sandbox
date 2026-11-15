use sandbox_shell::stream::{is_streamable, reads_standard_input, Filter};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn line_local_commands_stream() {
    assert!(is_streamable(&argv(&["cat"])));
    assert!(is_streamable(&argv(&["rev"])));
    assert!(is_streamable(&argv(&["tr", "a-z", "A-Z"])));
    assert!(is_streamable(&argv(&["grep", "-v", "x"])));
    assert!(is_streamable(&argv(&["sed", "-n", "s/a/b/p"])));
    assert!(is_streamable(&argv(&["cut", "-d:", "-f2"])));
    assert!(is_streamable(&argv(&["cut", "-d", ":", "-f", "2"])));
    assert!(is_streamable(&argv(&["head", "-n", "2"])));
    assert!(is_streamable(&argv(&["nl"])));
    assert!(is_streamable(&argv(&["nl", "-b", "a"])));
}

#[test]
fn commands_that_need_the_whole_input_or_a_file_do_not_stream() {
    assert!(!is_streamable(&argv(&["sort"])));
    assert!(!is_streamable(&argv(&["wc", "-l"])));
    assert!(!is_streamable(&argv(&["tail", "-n", "2"])));
    assert!(!is_streamable(&argv(&["cat", "file"])));
    assert!(
        !is_streamable(&argv(&["grep", "-c", "x"])),
        "a count needs every line"
    );
    assert!(
        !is_streamable(&argv(&["grep", "-n", "x"])),
        "a line number needs every line"
    );
    assert!(!is_streamable(&argv(&["grep", "x", "file"])));
    assert!(!is_streamable(&argv(&["tr", "-s", "a"])));
    assert!(!is_streamable(&argv(&["cut", "file"])), "cut needs a list");
    assert!(!is_streamable(&argv(&["head"])));
    assert!(!is_streamable(&argv(&["nl", "-v", "2"])));
}

#[test]
fn filters_start_with_their_counters() {
    assert_eq!(
        Filter::for_argv(&argv(&["head", "-n", "3"])),
        Some(Filter::Head { remaining: 3 })
    );
    assert_eq!(
        Filter::for_argv(&argv(&["nl"])),
        Some(Filter::Nl {
            all: false,
            number: 0
        })
    );
    assert_eq!(
        Filter::for_argv(&argv(&["nl", "-b", "a"])),
        Some(Filter::Nl {
            all: true,
            number: 0
        })
    );
    assert_eq!(Filter::for_argv(&argv(&["rev"])), Some(Filter::PerLine));
    assert_eq!(Filter::for_argv(&argv(&["sort"])), None);
}

#[test]
fn a_command_reads_standard_input_only_when_no_file_is_named() {
    assert!(reads_standard_input(&argv(&["sort"])));
    assert!(reads_standard_input(&argv(&["sort", "-r"])));
    assert!(reads_standard_input(&argv(&["cat"])));
    assert!(!reads_standard_input(&argv(&["cat", "file"])));
    assert!(
        reads_standard_input(&argv(&["head", "-n", "2"])),
        "the value of -n is not a file"
    );
    assert!(reads_standard_input(&argv(&["grep", "pattern"])));
    assert!(!reads_standard_input(&argv(&["grep", "pattern", "file"])));
    assert!(reads_standard_input(&argv(&["tr", "a", "b"])));
    assert!(
        reads_standard_input(&argv(&["tee", "out"])),
        "tee always reads its input"
    );
    assert!(!reads_standard_input(&argv(&["echo", "hi"])));
    assert!(
        !reads_standard_input(&argv(&["node", "script.js"])),
        "external tools keep their behaviour"
    );
}
