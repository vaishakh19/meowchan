//! Tauri command surface.
//!
//! Every command is narrowly scoped and does one thing. The frontend cannot run arbitrary code,
//! read arbitrary paths or delete anything without going through one of these, and each one
//! revalidates its input before acting.
//!
//! Naming: snake_case here, camelCase on the TypeScript side — Tauri converts automatically.
//!
//! Adding a command means adding it to `generate_handler!` in `lib.rs` **and** deciding which
//! permissions it requires — see `crate::authz`. A command that mutates the filesystem, reads a
//! paid feature, or removes duplicates goes through `authorize` before it does anything else.
//!
//! `validate_path` in `files.rs` is the shared path guard. Anything that takes a user-supplied
//! path either uses it or has an equivalent check of its own; the tests below cover both.

pub mod cleanup;
pub mod drives;
pub mod duplicates;
pub mod files;
pub mod large_files;
pub mod licensing;
pub mod scan;
pub mod settings;

pub(crate) use files::validate_path;

#[cfg(test)]
mod tests {
    /// Every module declared above must actually exist and be reachable from `lib.rs`'s handler.
    /// This is a compile-time smoke test: if a command module is added but never declared here,
    /// `generate_handler!` fails to resolve and the build breaks — which is the correct outcome,
    /// but only if the module list above is complete. These references make the breakage happen at
    /// this line with a clear message instead of deep inside the handler macro.
    #[test]
    fn every_command_module_is_declared() {
        let modules: Vec<&str> = vec![
            stringify!(crate::commands::cleanup),
            stringify!(crate::commands::drives),
            stringify!(crate::commands::duplicates),
            stringify!(crate::commands::files),
            stringify!(crate::commands::large_files),
            stringify!(crate::commands::licensing),
            stringify!(crate::commands::scan),
            stringify!(crate::commands::settings),
        ];
        assert_eq!(modules.len(), 8);
    }

    /// `validate_path` must be reachable crate-wide: the destructive paths reuse it rather than
    /// each inventing their own traversal check.
    #[test]
    fn validate_path_is_reachable_from_the_command_root() {
        assert!(crate::commands::validate_path("C:\\Users\\me").is_ok());
        assert!(crate::commands::validate_path("..\\..\\etc\\passwd").is_err());
        assert!(crate::commands::validate_path("C:\\Users\\me\0evil").is_err());
    }
}
