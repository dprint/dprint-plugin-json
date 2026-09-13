use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
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
/// are package names or similar are written alphabetically, one level deep. A dependency section
/// written in runs with a comment heading each is alphabetized a run at a time, with each heading
/// left over its run. Everything else is left alone: the order of `exports` conditions and of
/// `files`, `workspaces` or `scripts` entries is the author's to decide, and so is the order of
/// anything a package manager reads as a list of rules rather than as a map.
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

  // A comment written above a top level property travels with it. A conventional order rearranges
  // the whole file, so a comment left behind would end up over a property it says nothing about,
  // and one written above `dependencies` is almost always about those.
  root_object.sort_properties().by(compare_top_level_fields);
  for prop in root_object.properties() {
    let Some(section) = alphabetical_section(&decoded_name(&prop)) else {
      continue;
    };
    let Some(object) = prop.object_value() else {
      continue;
    };
    match section {
      Section::Plain => object.sort_properties().by(compare_properties),
      Section::Dependencies => sort_dependencies(&object),
      Section::Overrides if !names_a_package_twice(&object) => sort_dependencies(&object),
      Section::Overrides => {}
    }
  }

  let sorted = root.to_string();
  // an already conventional file is handed back without copying it
  if sorted == text {
    Cow::Borrowed(text)
  } else {
    Cow::Owned(sorted)
  }
}

/// Sorts a dependency section a run at a time.
///
/// A long list of dependencies is often written as runs with a comment heading each, and a
/// dependency belongs to its run rather than to the section as a whole, so no dependency is sorted
/// out of the run it was written in and each heading stays over its run.
///
/// Only a comment set off by a blank line starts a run. A blank line with nothing under it is just
/// spacing, and one the printer is free to remove: a section divided on it would come out unsorted
/// the first time it was formatted and sorted the second. A heading keeps its line, and the blank
/// line above it, however the section is printed.
///
/// The runs survive formatting but not `npm install`, which rewrites the dependency sections sorted
/// as a whole.
fn sort_dependencies(section: &CstObject) {
  let mut run = 0;
  let runs = section
    .properties()
    .iter()
    .map(|prop| {
      if has_heading(prop) {
        run += 1;
      }
      (prop.child_index(), run)
    })
    .collect::<HashMap<_, _>>();
  section.sort_properties().pin_comment_headers().by(|left, right| {
    runs
      .get(&left.child_index())
      .cmp(&runs.get(&right.child_index()))
      .then_with(|| compare_properties(left, right))
  });
}

/// Whether a comment set off by a blank line sits directly above the property on a line of its own,
/// which is what heads a run of dependencies.
fn has_heading(prop: &CstObjectProp) -> bool {
  let mut above = prop.previous_siblings().filter(|node| !node.is_whitespace());
  // walking upwards: the line break ending the comment's line, the comment, then the line break
  // before it, which rules out a comment trailing the previous property
  let comment_on_own_line = above.next().is_some_and(|node| node.is_newline())
    && above.next().is_some_and(|node| node.is_comment())
    && above.next().is_some_and(|node| node.is_newline());
  comment_on_own_line && prop.has_blank_line_before()
}

/// Whether two keys in `overrides` name the same package.
///
/// npm chooses between such keys by the order they were written in, taking the first whose version
/// matches, so sorting them could change what gets installed.
fn names_a_package_twice(overrides: &CstObject) -> bool {
  let mut seen = HashSet::new();
  overrides
    .properties()
    .iter()
    .any(|prop| !seen.insert(package_name(&decoded_name(prop)).to_string()))
}

/// The package a key names without any version written after it, so that `foo@^2` is `foo` and
/// `@scope/foo@^2` is `@scope/foo`.
fn package_name(key: &str) -> &str {
  // the first @ past the start is the one separating the version, so an alias like `foo@npm:bar@2`
  // names `foo`
  match key.get(1..).and_then(|rest| rest.find('@')) {
    Some(index) => &key[..index + 1],
    None => key,
  }
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

fn compare_properties(left: &CstObjectProp, right: &CstObjectProp) -> Ordering {
  compare_names(&decoded_name(left), &decoded_name(right))
}

/// Compares two names the way npm does, so that formatting doesn't undo npm's own sorting.
///
/// `npm install` rewrites the dependency sections sorted by `localeCompare(name, "en")`. That
/// ignores case until it has nothing else to go on and then puts the lower case first, and it
/// weighs punctuation differently from ASCII, putting `string_decoder` before `string-width` and
/// `@types/node` before `7zip-bin`. Comparing by byte would disagree on both, and the two tools
/// would take turns undoing each other. This reproduces the collation for the characters a package
/// name can contain.
fn compare_names(left: &str, right: &str) -> Ordering {
  fn weight(c: char) -> u32 {
    // the collation's order for the punctuation a package name can contain, all of which comes
    // before digits and letters
    match "_-.@*/+~".find(c) {
      Some(index) => index as u32,
      None => 0x100 + c.to_ascii_lowercase() as u32,
    }
  }

  fn weights(name: &str) -> impl Iterator<Item = u32> + '_ {
    name.chars().map(weight)
  }

  weights(left).cmp(weights(right)).then_with(|| right.cmp(left))
}

fn alphabetical_section(name: &str) -> Option<Section> {
  ALPHABETICAL_SECTIONS
    .iter()
    .find(|(section, _)| *section == name)
    .map(|(_, kind)| *kind)
}

fn field_index(name: &str) -> Option<usize> {
  FIELD_ORDER.iter().position(|field| *field == name)
}

/// The property's name with its escapes resolved, or an empty name when it can't be decoded.
fn decoded_name(prop: &CstObjectProp) -> String {
  prop.decoded_name().unwrap_or_default()
}

/// How a section whose properties are written in alphabetical order is sorted.
#[derive(Clone, Copy)]
enum Section {
  /// As a whole. Nobody writes `engines` or `bin` in runs, so there is nothing to keep together.
  Plain,
  /// A run at a time, as described on [`sort_dependencies`].
  Dependencies,
  /// Like dependencies, unless two keys name the same package, as described on
  /// [`names_a_package_twice`].
  Overrides,
}

/// The sections whose properties are written in alphabetical order, and how each is sorted.
///
/// Only maps keyed by a package name or similar appear here, where the order carries no meaning
/// beyond making an entry easy to find. `resolutions` is missing on purpose: yarn reads its keys as
/// patterns and takes the first that matches, so a specific pattern written above a broad one only
/// wins while it stays there.
const ALPHABETICAL_SECTIONS: &[(&str, Section)] = &[
  ("bin", Section::Plain),
  ("dependencies", Section::Dependencies),
  ("dependenciesMeta", Section::Dependencies),
  ("devDependencies", Section::Dependencies),
  ("engines", Section::Plain),
  ("optionalDependencies", Section::Dependencies),
  ("overrides", Section::Overrides),
  ("peerDependencies", Section::Dependencies),
  ("peerDependenciesMeta", Section::Dependencies),
];

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
    // an already conventional file is handed back without copying it
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
  fn leaves_resolutions_in_the_order_they_were_written() {
    // yarn takes the first pattern that matches, so the specific one only wins from above
    let text = r#"{ "resolutions": { "react-scripts/**/lodash": "4.17.21", "**/lodash": "4.17.15" } }"#;
    assert!(matches!(apply_conventions(text), Cow::Borrowed(_)));
  }

  #[test]
  fn leaves_overrides_alone_when_two_keys_name_one_package() {
    // npm takes the first key whose version matches, so this order decides what gets installed
    let text = r#"{ "overrides": { "foo@2": "2.9.9", "foo@1 || 2": "1.0.0" } }"#;
    assert!(matches!(apply_conventions(text), Cow::Borrowed(_)));
    // with nothing to choose between, the keys are just a map
    assert_eq!(
      apply_conventions(r#"{ "overrides": { "foo": "2", "bar": "1" } }"#),
      r#"{ "overrides": { "bar": "1", "foo": "2" } }"#
    );
  }

  #[test]
  fn reads_the_package_an_override_names() {
    assert_eq!(package_name("foo"), "foo");
    assert_eq!(package_name("foo@^2"), "foo");
    assert_eq!(package_name("@scope/foo"), "@scope/foo");
    assert_eq!(package_name("@scope/foo@^2"), "@scope/foo");
    assert_eq!(package_name("foo@npm:bar@2"), "foo");
    assert_eq!(package_name(""), "");
  }

  #[test]
  fn compares_names_the_way_npm_does() {
    // what `localeCompare(name, "en")` answers for these, which is what npm writes
    let mut names = vec![
      "zod",
      "js-yaml",
      "JSONStream",
      "jest",
      "@scope/b",
      "abc",
      "Abc",
      "string-width",
      "string_decoder",
      "7zip-bin",
      "@types/node",
    ];
    names.sort_by(|left, right| compare_names(left, right));
    assert_eq!(
      names,
      [
        "@scope/b",
        "@types/node",
        "7zip-bin",
        "abc",
        "Abc",
        "jest",
        "js-yaml",
        "JSONStream",
        "string_decoder",
        "string-width",
        "zod",
      ]
    );
  }

  #[test]
  fn every_alphabetical_section_is_a_known_field() {
    for (section, _) in ALPHABETICAL_SECTIONS {
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
