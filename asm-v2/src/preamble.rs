//! Generates `.equ` assembly constants from `#[account]` structs in lib.rs.
//!
//! For each struct annotated with `#[account]`:
//! - `StructName__SIZE` — `size_of::<Struct>()`
//! - `StructName__DISC_SIZE` — 8 (anchor discriminator)
//! - `StructName__INIT_SPACE` — 8 + size_of (total account allocation)
//! - `StructName__field` — byte offset of each field
//!
//! Offsets are computed from `#[repr(C)]` layout rules (which `#[account]`
//! enforces via bytemuck Pod). Fields must be primitive numeric types or
//! fixed-size arrays of them — no generics, no references.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use syn::{Meta, Token, punctuated::Punctuated};

/// Parse `lib.rs` and generate `.equ` preamble for all `#[account]` structs.
#[cfg_attr(not(test), allow(dead_code))]
pub fn generate(lib_rs: &Path) -> String {
    generate_tracked(lib_rs).0
}

/// Like [`generate`], but also returns every Rust source file that was parsed
/// while walking the module tree. Callers can use the returned paths to emit
/// `cargo:rerun-if-changed=` directives.
pub(crate) fn generate_tracked(lib_rs: &Path) -> (String, Vec<PathBuf>) {
    let root_dir = lib_rs.parent().unwrap_or_else(|| Path::new("."));
    let mut visited = HashSet::new();
    let output = generate_file(lib_rs, root_dir, &mut visited);
    let mut visited_files: Vec<_> = visited.into_iter().collect();
    visited_files.sort();
    (output, visited_files)
}

fn generate_file(path: &Path, module_dir: &Path, visited: &mut HashSet<PathBuf>) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return String::new();
    }

    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let file = match syn::parse_file(&source) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("anchor-asm: warning: can't parse {}: {e}", path.display());
            return String::new();
        }
    };

    let mut out = String::new();
    let source_dir = path.parent().unwrap_or_else(|| Path::new("."));
    visit_items(&file.items, source_dir, module_dir, visited, &mut out);
    out
}

fn visit_items(
    items: &[syn::Item],
    source_dir: &Path,
    module_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    out: &mut String,
) {
    for item in items {
        match item {
            syn::Item::Struct(s) if cfg_enabled(&s.attrs) && has_account_attr(s) => {
                if let Some(block) = emit_struct(s) {
                    out.push_str(&block);
                }
            }
            syn::Item::Mod(m) if cfg_enabled(&m.attrs) => {
                visit_module(m, source_dir, module_dir, visited, out)
            }
            _ => {}
        }
    }
}

fn visit_module(
    module: &syn::ItemMod,
    source_dir: &Path,
    module_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    out: &mut String,
) {
    if let Some((_, items)) = &module.content {
        let child_dir = inline_module_dir(source_dir, module_dir, module);
        visit_items(items, &child_dir, &child_dir, visited, out);
    } else if let Some((path, child_dir)) = resolve_module_file(source_dir, module_dir, module) {
        out.push_str(&generate_file(&path, &child_dir, visited));
    }
}

fn inline_module_dir(source_dir: &Path, module_dir: &Path, module: &syn::ItemMod) -> PathBuf {
    module_path_attr(source_dir, module)
        .map(|path| explicit_module_dir(&path))
        .unwrap_or_else(|| module_dir.join(module.ident.to_string()))
}

fn resolve_module_file(
    source_dir: &Path,
    module_dir: &Path,
    module: &syn::ItemMod,
) -> Option<(PathBuf, PathBuf)> {
    if let Some(path) = module_path_attr(source_dir, module) {
        return (path.exists() && path.is_file()).then(|| {
            let child_dir = explicit_module_dir(&path);
            (path, child_dir)
        });
    }

    let module_name = module.ident.to_string();
    let file = module_dir.join(format!("{module_name}.rs"));
    if file.exists() {
        return Some((file, module_dir.join(module_name)));
    }

    let mod_rs = module_dir.join(module_name).join("mod.rs");
    if mod_rs.exists() {
        let child_dir = mod_rs
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Some((mod_rs, child_dir))
    } else {
        None
    }
}

fn explicit_module_dir(path: &Path) -> PathBuf {
    if path.is_dir() || path.extension().is_none() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
    }
}

fn module_path_attr(source_dir: &Path, module: &syn::ItemMod) -> Option<PathBuf> {
    module
        .attrs
        .iter()
        .find_map(|attr| {
            let Meta::NameValue(nv) = &attr.meta else {
                return None;
            };
            if !nv.path.is_ident("path") {
                return None;
            }
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) = &nv.value
            else {
                return None;
            };
            Some(path.value())
        })
        .map(PathBuf::from)
        .map(|path| if path.is_absolute() { path } else { source_dir.join(path) })
}

/// Check if a struct should have assembly constants generated.
/// Matches `#[account]` (anchor v2) or `#[repr(C)]` (plain Pod).
fn has_account_attr(s: &syn::ItemStruct) -> bool {
    let account_attrs: Vec<_> = s
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("account"))
        .collect();
    let repr_attrs: Vec<_> = s
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("repr"))
        .collect();

    if !account_attrs.is_empty() {
        return account_attrs.len() == 1
            && is_plain_account_attr(account_attrs[0])
            && repr_attrs.is_empty();
    }
    repr_attrs.len() == 1 && is_exact_repr_c(repr_attrs[0])
}

fn is_plain_account_attr(attr: &syn::Attribute) -> bool {
    matches!(&attr.meta, syn::Meta::Path(path) if path.is_ident("account"))
}

fn is_exact_repr_c(attr: &syn::Attribute) -> bool {
    let syn::Meta::List(list) = &attr.meta else {
        return false;
    };
    if !attr.path().is_ident("repr") {
        return false;
    }

    let Ok(args) =
        list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
    else {
        return false;
    };

    args.len() == 1 && matches!(args.first(), Some(syn::Meta::Path(path)) if path.is_ident("C"))
}

fn cfg_enabled(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().filter(|attr| attr.path().is_ident("cfg")).all(eval_cfg_attr)
}

fn eval_cfg_attr(attr: &syn::Attribute) -> bool {
    let Ok(meta) = attr.parse_args::<Meta>() else {
        return false;
    };
    eval_cfg_meta(&meta)
}

fn eval_cfg_meta(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path
            .get_ident()
            .map(|ident| cfg_flag_is_set(&ident.to_string()))
            .unwrap_or(false),
        Meta::NameValue(nv) => {
            let Some(key) = nv.path.get_ident().map(|ident| ident.to_string()) else {
                return false;
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = &nv.value
            else {
                return false;
            };
            cfg_key_matches(&key, &value.value())
        }
        Meta::List(list) if list.path.is_ident("all") => parse_cfg_list(list)
            .map(|items| items.iter().all(eval_cfg_meta))
            .unwrap_or(false),
        Meta::List(list) if list.path.is_ident("any") => parse_cfg_list(list)
            .map(|items| items.iter().any(eval_cfg_meta))
            .unwrap_or(false),
        Meta::List(list) if list.path.is_ident("not") => parse_cfg_list(list)
            .map(|items| items.len() == 1 && !eval_cfg_meta(&items[0]))
            .unwrap_or(false),
        _ => false,
    }
}

fn parse_cfg_list(list: &syn::MetaList) -> Option<Vec<Meta>> {
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .ok()
        .map(|items| items.into_iter().collect())
}

fn cfg_flag_is_set(flag: &str) -> bool {
    let env_key = format!("CARGO_CFG_{}", cfg_env_name(flag));
    std::env::var_os(&env_key).is_some()
}

fn cfg_key_matches(key: &str, value: &str) -> bool {
    if key == "feature" {
        let feature_key = format!("CARGO_FEATURE_{}", cfg_env_name(value));
        if std::env::var_os(&feature_key).is_some() {
            return true;
        }
    }

    let env_key = format!("CARGO_CFG_{}", cfg_env_name(key));
    let Some(raw) = std::env::var_os(&env_key) else {
        return false;
    };
    raw.to_string_lossy()
        .split(',')
        .any(|entry| entry == value)
}

fn cfg_env_name(value: &str) -> String {
    value
        .replace('-', "_")
        .chars()
        .flat_map(|ch| ch.to_uppercase())
        .collect()
}

/// Emit `.equ` constants for a single struct.
fn emit_struct(s: &syn::ItemStruct) -> Option<String> {
    let name = &s.ident;
    let fields = match &s.fields {
        syn::Fields::Named(f) => &f.named,
        _ => return None,
    };

    let mut out = String::new();
    out.push_str(&format!("# {name} field offsets and sizes.\n"));
    out.push_str(&format!(
        "# {}\n",
        "-".repeat(70)
    ));

    // Compute repr(C) layout: fields in declaration order, each aligned
    // to its natural alignment, struct padded to max alignment at end.
    let mut offset: usize = 0;
    let mut max_align: usize = 1;

    for field in fields {
        if !cfg_enabled(&field.attrs) {
            continue;
        }

        let field_name = field.ident.as_ref()?;

        // Skip fields starting with _ (padding).
        let name_str = field_name.to_string();
        if name_str.starts_with('_') {
            let (size, align) = type_layout(&field.ty)?;
            offset = align_up(offset, align);
            offset += size;
            if align > max_align {
                max_align = align;
            }
            continue;
        }

        let (size, align) = type_layout(&field.ty)?;
        offset = align_up(offset, align);

        out.push_str(&format!(".equ {name}__{field_name}, {offset}\n"));

        offset += size;
        if align > max_align {
            max_align = align;
        }
    }

    // Pad to struct alignment.
    let struct_size = align_up(offset, max_align);

    out.push_str(&format!(".equ {name}__SIZE, {struct_size}\n"));
    out.push_str(&format!(".equ {name}__DISC_SIZE, 8\n"));
    out.push_str(&format!(
        ".equ {name}__INIT_SPACE, {}\n",
        8 + struct_size
    ));
    out.push_str(&format!(
        "# {}\n\n",
        "-".repeat(70)
    ));

    Some(out)
}

/// Returns (size, alignment) for a type, matching `#[repr(C)]` / Pod layout.
/// Only handles the types that make sense in `#[account]` structs.
fn type_layout(ty: &syn::Type) -> Option<(usize, usize)> {
    match ty {
        syn::Type::Path(tp) => {
            let seg = tp.path.segments.last()?;
            let name = seg.ident.to_string();
            match name.as_str() {
                "u8" | "i8" | "bool" => Some((1, 1)),
                "u16" | "i16" => Some((2, 2)),
                "u32" | "i32" | "f32" => Some((4, 4)),
                "u64" | "i64" | "f64" => Some((8, 8)),
                "u128" | "i128" => Some((16, 16)),
                // Anchor v2 Address = [u8; 32], alignment 1
                "Address" | "Pubkey" => Some((32, 1)),
                // PodBool = u8
                "PodBool" => Some((1, 1)),
                // Pod wrappers — alignment 1, stored as [u8; N]
                "PodU16" | "PodI16" => Some((2, 1)),
                "PodU32" | "PodI32" => Some((4, 1)),
                "PodU64" | "PodI64" => Some((8, 1)),
                "PodU128" | "PodI128" => Some((16, 1)),
                // PodVec<T, MAX> — need to inspect generic args
                "PodVec" => pod_vec_layout(&seg.arguments),
                _ => None,
            }
        }
        syn::Type::Array(arr) => {
            let (elem_size, elem_align) = type_layout(&arr.elem)?;
            let len = array_len(&arr.len)?;
            Some((elem_size * len, elem_align))
        }
        _ => None,
    }
}

/// Compute layout for `PodVec<T, MAX>`: `[len: PodU16 (2 bytes, align 1)][padding?][T; MAX]`.
fn pod_vec_layout(args: &syn::PathArguments) -> Option<(usize, usize)> {
    let syn::PathArguments::AngleBracketed(ab) = args else {
        return None;
    };
    let mut iter = ab.args.iter();

    // First arg: element type
    let syn::GenericArgument::Type(elem_ty) = iter.next()? else {
        return None;
    };
    let (elem_size, elem_align) = type_layout(elem_ty)?;

    // Second arg: MAX capacity (const generic)
    let max = match iter.next()? {
        syn::GenericArgument::Const(expr) => const_expr_value(expr)?,
        syn::GenericArgument::Type(syn::Type::Path(_)) => {
            // Could be a const path like MAX_SIGNERS — can't resolve,
            // skip this struct.
            return None;
        }
        _ => return None,
    };

    let data_offset = align_up(2, elem_align);
    let size = align_up(data_offset + elem_size * max, elem_align);
    Some((size, elem_align))
}

/// Extract a usize from a const expression (integer literal).
fn const_expr_value(expr: &syn::Expr) -> Option<usize> {
    if let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(i),
        ..
    }) = expr
    {
        i.base10_parse().ok()
    } else {
        None
    }
}

/// Extract array length from a const expression.
fn array_len(expr: &syn::Expr) -> Option<usize> {
    const_expr_value(expr)
}

fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("anchor-asm-v2-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_env_var<F>(key: &str, value: Option<&str>, f: F)
    where
        F: FnOnce(),
    {
        let _guard = env_lock();
        let previous = std::env::var_os(key);
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        f();
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn test_simple_struct() {
        let source = r#"
            #[account]
            pub struct Counter {
                pub value: u64,
                pub bump: u8,
                pub _pad: [u8; 7],
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_lib.rs");
        std::fs::write(&tmp, source).unwrap();
        let result = generate(&tmp);
        assert!(result.contains(".equ Counter__value, 0"));
        assert!(result.contains(".equ Counter__bump, 8"));
        assert!(result.contains(".equ Counter__SIZE, 16"));
        assert!(result.contains(".equ Counter__INIT_SPACE, 24"));
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_address_field() {
        let source = r#"
            #[account]
            pub struct Config {
                pub admin: Address,
                pub bump: u8,
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_addr.rs");
        std::fs::write(&tmp, source).unwrap();
        let result = generate(&tmp);
        // Address is 32 bytes, align 1
        assert!(result.contains(".equ Config__admin, 0"));
        assert!(result.contains(".equ Config__bump, 32"));
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_account_attr_must_be_plain_or_exact_repr_c() {
        let source = r#"
            #[account]
            pub struct ZeroCopy {
                pub value: u64,
            }

            #[account(borsh)]
            pub struct BorshBacked {
                pub value: u64,
            }

            #[account(zero_copy)]
            pub struct MacroArgs {
                pub value: u64,
            }

            #[account(borsh)]
            #[repr(C)]
            pub struct BorshReprC {
                pub value: u64,
            }

            #[account]
            #[repr(C)]
            pub struct ZeroCopyWithUserReprC {
                pub value: u64,
            }

            #[account]
            #[repr(packed)]
            pub struct ZeroCopyPacked {
                pub value: u64,
            }

            #[account]
            #[repr(align(8))]
            pub struct ZeroCopyAligned {
                pub value: u64,
            }

            #[repr(C)]
            pub struct PlainPod {
                pub value: u64,
            }

            #[repr(packed)]
            pub struct PackedPod {
                pub value: u64,
            }

            #[repr(transparent)]
            pub struct TransparentPod {
                pub value: u64,
            }

            #[repr(C, packed)]
            pub struct MixedReprPod {
                pub value: u64,
            }

            #[repr(C)]
            #[repr(align(8))]
            pub struct SplitAlignedPod {
                pub value: u64,
            }

            #[repr(C)]
            #[repr(packed)]
            pub struct SplitPackedPod {
                pub value: u64,
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_attrs.rs");
        std::fs::write(&tmp, source).unwrap();
        let result = generate(&tmp);
        assert!(result.contains(".equ ZeroCopy__value, 0"));
        assert!(result.contains(".equ PlainPod__value, 0"));
        assert!(!result.contains("BorshBacked__value"));
        assert!(!result.contains("MacroArgs__value"));
        assert!(!result.contains("BorshReprC__value"));
        assert!(!result.contains("ZeroCopyWithUserReprC__value"));
        assert!(!result.contains("ZeroCopyPacked__value"));
        assert!(!result.contains("ZeroCopyAligned__value"));
        assert!(!result.contains("PackedPod__value"));
        assert!(!result.contains("TransparentPod__value"));
        assert!(!result.contains("MixedReprPod__value"));
        assert!(!result.contains("SplitAlignedPod__value"));
        assert!(!result.contains("SplitPackedPod__value"));
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_generate_respects_cfg_gated_fields() {
        let source = r#"
            #[repr(C)]
            pub struct CfgFieldLayout {
                pub tag: u8,
                #[cfg(feature = "asm_cfg_field")]
                pub gated: u64,
                pub bump: u8,
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_cfg_field.rs");
        std::fs::write(&tmp, source).unwrap();

        with_env_var("CARGO_FEATURE_ASM_CFG_FIELD", None, || {
            let result = generate(&tmp);
            assert!(result.contains(".equ CfgFieldLayout__tag, 0"));
            assert!(result.contains(".equ CfgFieldLayout__bump, 1"));
            assert!(result.contains(".equ CfgFieldLayout__SIZE, 2"));
            assert!(!result.contains("CfgFieldLayout__gated"));
        });

        with_env_var("CARGO_FEATURE_ASM_CFG_FIELD", Some("1"), || {
            let result = generate(&tmp);
            assert!(result.contains(".equ CfgFieldLayout__tag, 0"));
            assert!(result.contains(".equ CfgFieldLayout__gated, 8"));
            assert!(result.contains(".equ CfgFieldLayout__bump, 16"));
            assert!(result.contains(".equ CfgFieldLayout__SIZE, 24"));
        });

        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_generate_recurses_inline_and_file_backed_modules() {
        let dir = temp_test_dir("mods");
        let lib_rs = dir.join("lib.rs");
        let outer_dir = dir.join("outer");
        std::fs::create_dir_all(&outer_dir).unwrap();

        std::fs::write(
            &lib_rs,
            r#"
            mod outer {
                pub mod inline_leaf {
                    #[account]
                    pub struct NestedInline {
                        pub value: u64,
                    }

                    #[account]
                    #[repr(packed)]
                    pub struct NestedPacked {
                        pub value: u64,
                    }
                }

                pub mod file_leaf;
            }

            mod state;
            "#,
        )
        .unwrap();
        std::fs::write(
            outer_dir.join("file_leaf.rs"),
            r#"
            #[repr(C)]
            pub struct NestedFile {
                pub value: u64,
            }

            #[account(borsh)]
            pub struct NestedBorsh {
                pub value: u64,
            }
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.join("state.rs"),
            r#"
            #[account]
            pub struct RootFile {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let result = generate(&lib_rs);
        assert!(result.contains(".equ NestedInline__value, 0"));
        assert!(result.contains(".equ NestedFile__value, 0"));
        assert!(result.contains(".equ RootFile__value, 0"));
        assert!(!result.contains("NestedPacked__value"));
        assert!(!result.contains("NestedBorsh__value"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_respects_cfg_on_nested_modules() {
        let dir = temp_test_dir("cfg-mod");
        let lib_rs = dir.join("lib.rs");

        std::fs::write(
            &lib_rs,
            r#"
            #[cfg(feature = "asm_cfg_module")]
            mod gated {
                #[account]
                pub struct NestedEnabled {
                    pub value: u64,
                }
            }
            "#,
        )
        .unwrap();

        with_env_var("CARGO_FEATURE_ASM_CFG_MODULE", None, || {
            let result = generate(&lib_rs);
            assert!(!result.contains("NestedEnabled__value"));
        });

        with_env_var("CARGO_FEATURE_ASM_CFG_MODULE", Some("1"), || {
            let result = generate(&lib_rs);
            assert!(result.contains(".equ NestedEnabled__value, 0"));
        });

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_resolves_mod_rs_modules() {
        let dir = temp_test_dir("mod-rs");
        let lib_rs = dir.join("lib.rs");
        let outer_dir = dir.join("outer");
        std::fs::create_dir_all(&outer_dir).unwrap();

        std::fs::write(&lib_rs, "mod outer;\n").unwrap();
        std::fs::write(
            outer_dir.join("mod.rs"),
            r#"
            #[repr(C)]
            pub struct NestedFromModRs {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let result = generate(&lib_rs);
        assert!(result.contains(".equ NestedFromModRs__value, 0"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_resolves_path_attr_modules() {
        let dir = temp_test_dir("path-attr");
        let lib_rs = dir.join("lib.rs");
        let alt_dir = dir.join("alt");
        std::fs::create_dir_all(&alt_dir).unwrap();

        std::fs::write(&lib_rs, "#[path = \"alt/custom_state.rs\"] mod state;\n").unwrap();
        std::fs::write(
            alt_dir.join("custom_state.rs"),
            r#"
            #[repr(C)]
            pub struct PathAttrState {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let result = generate(&lib_rs);
        assert!(result.contains(".equ PathAttrState__value, 0"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_resolves_path_attr_relative_to_non_mod_rs_declaring_file() {
        let dir = temp_test_dir("path-attr-non-mod-rs");
        let lib_rs = dir.join("lib.rs");

        std::fs::write(&lib_rs, "mod state;\n").unwrap();
        std::fs::write(
            dir.join("state.rs"),
            r#"
            #[path = "custom.rs"]
            mod child;
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.join("custom.rs"),
            r#"
            #[repr(C)]
            pub struct CustomState {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let result = generate(&lib_rs);
        assert!(result.contains(".equ CustomState__value, 0"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_resolves_nested_path_attrs_inside_inline_path_modules() {
        let dir = temp_test_dir("inline-path-attr");
        let lib_rs = dir.join("lib.rs");
        let thread_files_dir = dir.join("thread_files");
        std::fs::create_dir_all(&thread_files_dir).unwrap();

        std::fs::write(
            &lib_rs,
            r#"
            #[path = "thread_files"]
            mod thread {
                #[path = "tls.rs"]
                mod local_data;
            }
            "#,
        )
        .unwrap();
        std::fs::write(
            thread_files_dir.join("tls.rs"),
            r#"
            #[repr(C)]
            pub struct InlinePathState {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let result = generate(&lib_rs);
        assert!(result.contains(".equ InlinePathState__value, 0"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_generate_tracks_nested_module_dependencies() {
        let dir = temp_test_dir("tracked-deps");
        let lib_rs = dir.join("lib.rs");
        let state_rs = dir.join("state.rs");
        let custom_rs = dir.join("custom.rs");

        std::fs::write(&lib_rs, "mod state;\n").unwrap();
        std::fs::write(
            &state_rs,
            r#"
            #[path = "custom.rs"]
            mod child;
            "#,
        )
        .unwrap();
        std::fs::write(
            &custom_rs,
            r#"
            #[repr(C)]
            pub struct NestedTracked {
                pub value: u64,
            }
            "#,
        )
        .unwrap();

        let (result, visited_files) = generate_tracked(&lib_rs);
        assert!(result.contains(".equ NestedTracked__value, 0"));

        let canon = |path: &Path| std::fs::canonicalize(path).unwrap();
        assert!(visited_files.contains(&canon(&lib_rs)));
        assert!(visited_files.contains(&canon(&state_rs)));
        assert!(visited_files.contains(&canon(&custom_rs)));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_pod_vec_layout_respects_element_alignment() {
        let source = r#"
            #[repr(C)]
            pub struct Holder {
                pub prefix: u8,
                pub values: PodVec<u16, 1>,
                pub suffix: u8,
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_pod_vec.rs");
        std::fs::write(&tmp, source).unwrap();
        let result = generate(&tmp);
        assert!(result.contains(".equ Holder__prefix, 0"));
        assert!(result.contains(".equ Holder__values, 2"));
        assert!(result.contains(".equ Holder__suffix, 6"));
        assert!(result.contains(".equ Holder__SIZE, 8"));
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_pod_vec_layout_stays_padding_free_for_align1_elements() {
        let source = r#"
            #[repr(C)]
            pub struct ByteHolder {
                pub prefix: u8,
                pub values: PodVec<u8, 1>,
                pub suffix: u8,
            }
        "#;
        let tmp = std::env::temp_dir().join("anchor_asm_test_pod_vec_u8.rs");
        std::fs::write(&tmp, source).unwrap();
        let result = generate(&tmp);
        assert!(result.contains(".equ ByteHolder__prefix, 0"));
        assert!(result.contains(".equ ByteHolder__values, 1"));
        assert!(result.contains(".equ ByteHolder__suffix, 4"));
        assert!(result.contains(".equ ByteHolder__SIZE, 5"));
        std::fs::remove_file(tmp).ok();
    }
}
