//! Minify the HTML pages

use std::{
	env, fs,
	path::{Path, PathBuf},
};

use minify_html::Cfg;

fn main() {
	minify("bug", "assets/bug.html");
	minify("index", "assets/index.html");
}

/// Minify the HTML file named `name` at `path`
fn minify(name: &str, path: impl AsRef<Path>) {
	let path = path.as_ref();

	println!("cargo::rerun-if-changed={}", path.to_str().unwrap());

	let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap())
		.join(name)
		.with_extension("html");
	let html = fs::read_to_string(path).unwrap();

	let config = Cfg {
		minify_doctype: false,
		allow_noncompliant_unquoted_attribute_values: false,
		keep_closing_tags: false,
		keep_html_and_head_opening_tags: false,
		allow_removing_spaces_between_attributes: false,
		keep_comments: false,
		minify_css: true,
		// TODO: re-enabled this, currently needed as a workaround for a minify_js bug:
		minify_js: false,
		remove_bangs: true,
		remove_processing_instructions: true,
		..Cfg::default()
	};

	let minified = minify_html::minify(html.as_bytes(), &config);

	fs::write(out_path, minified).unwrap();
}
