use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::Path;

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::CstObject;
use jsonc_parser::cst::CstObjectProp;
use jsonc_parser::cst::CstRootNode;

pub fn is_package_json_file(path: &Path) -> bool {
  // no need to worry about different casing because npm only ever reads a file named exactly
  // package.json (https://docs.npmjs.com/cli/configuring-npm/package-json)
  path.file_name().map(|n| n == "package.json").unwrap_or(false)
}

/// Rewrites a `package.json` with its properties in the conventional order.
///
/// The top level is written in the conventional field order, which is the one used by
/// [`sort-package-json`](https://github.com/keithamus/sort-package-json), and the maps whose keys
/// are package names or similar are written alphabetically, one level deep. Everything else is
/// left alone: the order of `exports` conditions and of `files`, `workspaces` or `scripts` entries
/// is the author's to decide.
///
/// The work happens on the CST so that what was written with a property travels with it. Text that
/// doesn't parse is handed back untouched, since the formatter is about to report that itself.
pub fn apply_conventions(text: &str) -> Cow<'_, str> {
  let Ok(root) = CstRootNode::parse(text, &ParseOptions::default()) else {
    return Cow::Borrowed(text);
  };
  let Some(root_object) = root.object_value() else {
    return Cow::Borrowed(text);
  };

  sort_properties(&root_object, compare_top_level_fields);
  for prop in root_object.properties() {
    if ALPHABETICAL_SECTIONS.contains(&prop.decoded_name().unwrap_or_default().as_str())
      && let Some(section) = prop.object_value()
    {
      sort_properties(&section, |left, right| {
        compare_names(&decoded_name(left), &decoded_name(right))
      });
    }
  }

  let sorted = root.to_string();
  // a file already written in the conventional order comes back exactly as it was, so the
  // formatter sees the author's own text rather than a copy of it
  if sorted == text {
    Cow::Borrowed(text)
  } else {
    Cow::Owned(sorted)
  }
}

/// The sections whose properties are written in alphabetical order.
///
/// Only maps keyed by a package name or similar appear here, where the order carries no meaning
/// beyond making an entry easy to find.
const ALPHABETICAL_SECTIONS: &[&str] = &[
  "bin",
  "dependencies",
  "dependenciesMeta",
  "devDependencies",
  "engines",
  "optionalDependencies",
  "overrides",
  "peerDependencies",
  "peerDependenciesMeta",
  "resolutions",
];

fn sort_properties(obj: &CstObject, compare: impl FnMut(&CstObjectProp, &CstObjectProp) -> Ordering) {
  // The comments above a property travel with it rather than staying put. A conventional order
  // rearranges the whole file, so a comment left behind would end up over a property it says
  // nothing about, and a comment written above `dependencies` is almost always about those.
  obj.sort_properties().by(compare)
}

fn compare_top_level_fields(left: &CstObjectProp, right: &CstObjectProp) -> Ordering {
  let left = decoded_name(left);
  let right = decoded_name(right);
  match (field_index(&left), field_index(&right)) {
    (Some(left), Some(right)) => left.cmp(&right),
    (Some(_), None) => Ordering::Less,
    (None, Some(_)) => Ordering::Greater,
    // a field the conventions don't know goes below the ones they do, in alphabetical order, with
    // the underscore prefixed fields npm adds to an installed package (`_id`, `_resolved`, ...) last
    (None, None) => left
      .starts_with('_')
      .cmp(&right.starts_with('_'))
      .then_with(|| compare_names(&left, &right)),
  }
}

/// Compares two names the way npm does, so that formatting doesn't undo npm's own sorting.
///
/// `npm install` rewrites the dependency sections sorted by `localeCompare(name, "en")`, which
/// ignores case until it has nothing else to go on and then puts the lower case first. Comparing
/// by byte instead would file every capitalised package (`JSONStream`, `Base64`) above all the
/// lower case ones, and the two tools would take turns undoing each other. This is only an
/// approximation of the full collation -- it doesn't reproduce how punctuation is weighed -- but
/// it agrees with npm on the names that actually differ.
fn compare_names(left: &str, right: &str) -> Ordering {
  fn lowercase(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().map(|c| c.to_ascii_lowercase())
  }

  lowercase(left).cmp(lowercase(right)).then_with(|| right.cmp(left))
}

fn field_index(name: &str) -> Option<usize> {
  FIELD_ORDER.iter().position(|field| *field == name)
}

/// A property whose name can't be decoded sorts as if it had none, which puts it above the rest.
fn decoded_name(prop: &CstObjectProp) -> String {
  prop.decoded_name().unwrap_or_default()
}

/// The conventional order of the top level fields, as used by `sort-package-json`.
const FIELD_ORDER: &[&str] = &[
  "$schema",
  "name",
  "displayName",
  "version",
  "stableVersion",
  "private",
  "description",
  "categories",
  "keywords",
  "homepage",
  "bugs",
  "repository",
  "funding",
  "license",
  "qna",
  "author",
  "maintainers",
  "contributors",
  "publisher",
  "sideEffects",
  "type",
  "imports",
  "exports",
  "main",
  "svelte",
  "umd:main",
  "jsdelivr",
  "unpkg",
  "module",
  "source",
  "jsnext:main",
  "browser",
  "react-native",
  "types",
  "typesVersions",
  "typings",
  "style",
  "example",
  "examplestyle",
  "assets",
  "bin",
  "man",
  "directories",
  "files",
  "workspaces",
  "binary",
  "scripts",
  "betterScripts",
  "wireit",
  "l10n",
  "contributes",
  "activationEvents",
  "husky",
  "simple-git-hooks",
  "pre-commit",
  "commitlint",
  "lint-staged",
  "nano-staged",
  "config",
  "nodemonConfig",
  "browserify",
  "babel",
  "browserslist",
  "xo",
  "prettier",
  "eslintConfig",
  "eslintIgnore",
  "npmpkgjsonlint",
  "npmPackageJsonLintConfig",
  "npmpackagejsonlint",
  "release",
  "remarkConfig",
  "stylelint",
  "ava",
  "jest",
  "jest-junit",
  "jest-stare",
  "mocha",
  "nyc",
  "c8",
  "tap",
  "oclif",
  "resolutions",
  "overrides",
  "dependencies",
  "devDependencies",
  "dependenciesMeta",
  "peerDependencies",
  "peerDependenciesMeta",
  "optionalDependencies",
  "bundledDependencies",
  "bundleDependencies",
  "extensionPack",
  "extensionDependencies",
  "flat",
  "packageManager",
  "engines",
  "engineStrict",
  "devEngines",
  "volta",
  "languageName",
  "os",
  "cpu",
  "preferGlobal",
  "publishConfig",
  "icon",
  "badges",
  "galleryBanner",
  "preview",
  "markdown",
  "pnpm",
];

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::*;

  #[test]
  fn test_is_package_json_file() {
    assert!(is_package_json_file(&PathBuf::from("package.json")));
    assert!(is_package_json_file(&PathBuf::from("/test/package.json")));
    assert!(!is_package_json_file(&PathBuf::from("/test/Package.json")));
    assert!(!is_package_json_file(&PathBuf::from("/test/mypackage.json")));
    assert!(!is_package_json_file(&PathBuf::from("/test/package-lock.json")));
    assert!(!is_package_json_file(&PathBuf::from("/test/package.jsonc")));
    if cfg!(windows) {
      assert!(is_package_json_file(&PathBuf::from(r"test\package.json")));
    }
  }

  #[test]
  fn leaves_text_it_does_not_rearrange_exactly_as_it_was() {
    // the borrow is load bearing: nothing is rewritten unless the conventions actually move something
    for text in [
      r#"{ "name": "a", "dependencies": { "a": "1", "b": "2" } }"#,
      "{}",
      r#"{ "zzz": 1 }"#,
      "[3, 1, 2]",
      "5",
      "",
      "{ not json",
      // a section that isn't an object has no properties to sort
      r#"{ "name": "a", "dependencies": ["b", "a"] }"#,
    ] {
      assert!(matches!(apply_conventions(text), Cow::Borrowed(_)), "rewrote: {}", text);
    }
  }

  #[test]
  fn orders_the_root_and_its_sections() {
    assert_eq!(
      apply_conventions(r#"{ "version": "1", "name": "a" }"#),
      r#"{ "name": "a", "version": "1" }"#
    );
    assert_eq!(
      apply_conventions(r#"{ "dependencies": { "b": "1", "a": "2" }, "name": "a" }"#),
      r#"{ "name": "a", "dependencies": { "a": "2", "b": "1" } }"#
    );
  }

  #[test]
  fn compares_names_the_way_npm_does() {
    // what `localeCompare(name, "en")` answers for these, which is what npm writes
    let mut names = vec!["zod", "js-yaml", "JSONStream", "jest", "@scope/b", "abc", "Abc"];
    names.sort_by(|left, right| compare_names(left, right));
    assert_eq!(
      names,
      ["@scope/b", "abc", "Abc", "jest", "js-yaml", "JSONStream", "zod"]
    );
  }

  #[test]
  fn every_alphabetical_section_is_a_known_field() {
    for section in ALPHABETICAL_SECTIONS {
      assert!(FIELD_ORDER.contains(section), "{} is missing from FIELD_ORDER", section);
    }
  }

  #[test]
  fn no_field_is_listed_twice() {
    // a repeat would make the second slot dead, since `field_index` answers with the first
    let mut seen = FIELD_ORDER.to_vec();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), FIELD_ORDER.len());
  }
}
