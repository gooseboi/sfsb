#![feature(exit_status_error)]

use std::{path::{Path, PathBuf}, process::Command};

fn main() {
    let templates_dir = "templates";
    let output_dir = "built_templates";
    println!("cargo:rerun-if-changed={templates_dir}/");
    let templates_dir = Path::new(templates_dir);
    let output_dir = Path::new(output_dir);

    std::fs::create_dir_all(&output_dir).unwrap();
    for t in templates_dir.read_dir().unwrap() {
        let template_path = t.unwrap().path();
        let template_file = template_path.file_name().unwrap();
        let out_file = {
            let mut p = PathBuf::from(&output_dir);
            p.push(&template_file);
            p
        };
        Command::new("minhtml").args(&["--do-not-minify-doctype", "--keep-html-and-head-opening-tags", "--preserve-brace-template-syntax", "--minify-css"])
                               .arg(&template_path)
                               .arg("--output")
                               .arg(&out_file)
                               .status()
                               .unwrap()
                               .exit_ok()
                               .unwrap();
    }
}
