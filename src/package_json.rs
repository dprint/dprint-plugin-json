use std::cmp::Ordering;
use std::path::Path;

use jsonc_parser::ast::Object;
use jsonc_parser::ast::ObjectProp;
use jsonc_parser::ast::ObjectPropName;
use jsonc_parser::ast::Value;

use crate::generation::PropertyOrders;

pub fn is_package_json_file(path: &Path) -> bool {
  // no need to worry about different casing because npm only ever reads a file named exactly
  // package.json (https://docs.npmjs.com/cli/configuring-npm/package-json)
  path.file_name().map(|n| n == "package.json").unwrap_or(false)
}

/// Works out the order the `package.json` conventions put each object's properties in.
///
/// The top level is written in the conventional field order, which is the one used by
/// [`sort-package-json`](https://github.com/keithamus/sort-package-json), and the maps whose keys
/// are package names or similar are written alphabetically, one level deep. Everything else is
/// left alone: the order of `exports` conditions and of `files`, `workspaces` or `scripts` entries
/// is the author's to decide.
pub fn property_orders(value: &Value) -> PropertyOrders {
  let mut orders = PropertyOrders::new();
  let Value::Object(root) = value else {
    return orders;
  };

  insert_order(&mut orders, root, compare_top_level_fields);
  for prop in &root.properties {
    if ALPHABETICAL_SECTIONS.contains(&prop_name(prop))
      && let Value::Object(section) = &prop.value
    {
      insert_order(&mut orders, section, |left, right| {
        compare_names(prop_name(left), prop_name(right))
      });
    }
  }

  orders
}

/// Records the order to write `obj`'s properties in, unless that's the order they're already in.
fn insert_order(orders: &mut PropertyOrders, obj: &Object, compare: impl Fn(&ObjectProp, &ObjectProp) -> Ordering) {
  let mut order = (0..obj.properties.len()).collect::<Vec<_>>();
  // a stable sort leaves two properties sharing a name in the order they were written
  order.sort_by(|left, right| compare(&obj.properties[*left], &obj.properties[*right]));
  if order.iter().enumerate().any(|(index, sorted)| index != *sorted) {
    orders.insert(obj.range.start, order);
  }
}

fn compare_top_level_fields(left: &ObjectProp, right: &ObjectProp) -> Ordering {
  let left = prop_name(left);
  let right = prop_name(right);
  match (field_index(left), field_index(right)) {
    (Some(left), Some(right)) => left.cmp(&right),
    (Some(_), None) => Ordering::Less,
    (None, Some(_)) => Ordering::Greater,
    // a field the conventions don't know goes below the ones they do, in alphabetical order, with
    // the underscore prefixed fields npm adds to an installed package (`_id`, `_resolved`, ...) last
    (None, None) => left
      .starts_with('_')
      .cmp(&right.starts_with('_'))
      .then_with(|| compare_names(left, right)),
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

fn prop_name<'a>(prop: &'a ObjectProp<'_>) -> &'a str {
  match &prop.name {
    ObjectPropName::String(name) => name.value.as_ref(),
    ObjectPropName::Word(name) => name.value,
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

  use jsonc_parser::ParseResult;
  use jsonc_parser::parse_to_ast;

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
  fn orders_only_the_objects_it_rearranges() {
    let empty: Vec<Vec<usize>> = Vec::new();
    // the emptiness is load bearing: an object with no entry keeps its blank lines
    assert_eq!(
      orders_of(r#"{ "name": "a", "dependencies": { "a": "1", "b": "2" } }"#),
      empty
    );
    assert_eq!(orders_of("{}"), empty);
    assert_eq!(orders_of(r#"{ "zzz": 1 }"#), empty);
    assert_eq!(orders_of("[3, 1, 2]"), empty);
    assert_eq!(orders_of(r#"{ "version": "1", "name": "a" }"#), vec![vec![1, 0]]);
    // the root and the section are ordered separately
    assert_eq!(
      orders_of(r#"{ "dependencies": { "b": "1", "a": "2" }, "name": "a" }"#),
      vec![vec![1, 0], vec![1, 0]]
    );
    // a section that isn't an object is left alone
    assert_eq!(orders_of(r#"{ "name": "a", "dependencies": ["b", "a"] }"#), empty);
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

  /// The orders worked out for `text`, sorted so the assertions don't depend on the map's iteration order.
  fn orders_of(text: &str) -> Vec<Vec<usize>> {
    let ParseResult { value, .. } = parse_to_ast(text, &Default::default(), &Default::default()).unwrap();
    let orders = property_orders(&value.unwrap());
    let mut orders = orders.orders().map(|order| order.to_vec()).collect::<Vec<_>>();
    orders.sort();
    orders
  }
}
