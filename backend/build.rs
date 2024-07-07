use std::{
	env,
	error::Error,
	fs,
	path::{Path, PathBuf},
};

use minify_html::Cfg;

fn main() -> Result<(), Box<dyn Error>> {
	minify("bug", "./bug.html");

	Ok(())
}

fn minify(name: &str, path: impl AsRef<Path>) {
	let path = path.as_ref();

	println!("cargo::rerun-if-changed={}", path.to_str().unwrap());

	let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap())
		.join(name)
		.with_extension("html");
	let html = fs::read_to_string(path).unwrap();

	let config = Cfg {
		do_not_minify_doctype: true,
		ensure_spec_compliant_unquoted_attribute_values: true,
		keep_closing_tags: false,
		keep_html_and_head_opening_tags: false,
		keep_spaces_between_attributes: true,
		keep_comments: false,
		minify_css: true,
		minify_js: true,
		remove_bangs: true,
		remove_processing_instructions: true,
		..Cfg::default()
	};

	let minified = minify_html::minify(html.as_bytes(), &config);

	fs::write(out_path, minified).unwrap();
}
