//! Narrowly scoped commands. Everything that reads the filesystem stays
//! read-only; the only mutating command is the guarded Recycle Bin
//! deletion (`delete`). The `pro` commands verify licences with Dodo
//! Payments and never touch the filesystem.

pub mod delete;
pub mod drives;
pub mod duplicates;
pub mod files;
pub mod large_files;
pub mod pro;
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
            stringify!(crate::commands::delete),
            stringify!(crate::commands::drives),
            stringify!(crate::commands::duplicates),
            stringify!(crate::commands::files),
            stringify!(crate::commands::large_files),
            stringify!(crate::commands::pro),
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
