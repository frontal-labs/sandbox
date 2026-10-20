//! TypeScript for `node FILE.ts` (PLAN.md §9 M1).
//!
//! The file is parsed as TypeScript, and swc's TypeScript transform removes the
//! types and lowers the constructs that have runtime code, such as enums and
//! parameter properties. The output is plain JavaScript, which runs as a classic
//! script like any `.js` file. Types are not checked, so a type error runs. Modules
//! (`import`, `export`) are refused by the script runner, as they are for `.js`.

use sandbox_types::{ErrorCode, SandboxError};
use swc_core::common::{sync::Lrc, FileName, Globals, Mark, SourceMap, GLOBALS};
use swc_core::ecma::ast::{EsVersion, Program};
use swc_core::ecma::codegen::{text_writer::JsWriter, Config, Emitter};
use swc_core::ecma::parser::{parse_file_as_module, Syntax, TsSyntax};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::transforms::typescript::strip;
use swc_core::ecma::visit::VisitMutWith;

/// Erases the TypeScript types in `source` and returns JavaScript. `file_name` is
/// used only in error messages.
pub fn strip_types(source: &str, file_name: &str) -> Result<String, SandboxError> {
    let cm: Lrc<SourceMap> = Default::default();
    let file = cm.new_source_file(
        Lrc::new(FileName::Custom(file_name.to_string())),
        source.to_string(),
    );
    let mut errors = Vec::new();
    let module = parse_file_as_module(
        &file,
        Syntax::Typescript(TsSyntax::default()),
        EsVersion::latest(),
        None,
        &mut errors,
    )
    .map_err(|e| syntax_error(file_name, &format!("{:?}", e.kind())))?;
    if let Some(first) = errors.first() {
        return Err(syntax_error(file_name, &format!("{:?}", first.kind())));
    }

    GLOBALS.set(&Globals::new(), || {
        let unresolved = Mark::new();
        let top_level = Mark::new();
        let mut program = Program::Module(module);
        program.visit_mut_with(&mut resolver(unresolved, top_level, true));
        program.mutate(&mut strip(unresolved, top_level));
        let Program::Module(module) = program else {
            unreachable!("a module stays a module through type stripping");
        };
        let mut output = Vec::new();
        {
            let mut emitter = Emitter {
                cfg: Config::default(),
                cm: cm.clone(),
                comments: None,
                wr: JsWriter::new(cm.clone(), "\n", &mut output, None),
            };
            emitter.emit_module(&module).map_err(|e| {
                SandboxError::new(
                    ErrorCode::RuntimeError,
                    format!("{file_name}: cannot emit JavaScript: {e}"),
                )
            })?;
        }
        String::from_utf8(output).map_err(|e| {
            SandboxError::new(
                ErrorCode::RuntimeError,
                format!("{file_name}: emitted code is not UTF-8: {e}"),
            )
        })
    })
}

fn syntax_error(file_name: &str, detail: &str) -> SandboxError {
    SandboxError::new(
        ErrorCode::RuntimeError,
        format!("{file_name}: TypeScript syntax error ({detail})"),
    )
}
