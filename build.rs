use html_minifier::HTMLMinifier;
use std::{
    env,
    fs::{self, File},
    io::{BufWriter, Read as _, Write as _},
    path::Path,
};

fn minify_template(template_path: &Path, output_dir: &Path) {
    let template_filename = template_path.file_name().unwrap();
    let out_path = output_dir.join(template_filename);

    let mut input_file = File::open(template_path).unwrap();
    let output_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(out_path)
        .unwrap();

    let mut html_minifier = HTMLMinifier::new();
    let mut buffer = [0u8; 1024];

    loop {
        let c = input_file.read(&mut buffer).unwrap();

        if c == 0 {
            break;
        }

        html_minifier.digest(&buffer[..c]).unwrap();
    }
    let mut writer = BufWriter::new(output_file);
    for c in html_minifier.get_html().iter().filter(|c| **c != b'\n') {
        writer.write_all(&[*c]).unwrap();
    }
}

fn main() {
    let output_dir = env::var("OUT_DIR").unwrap();
    let output_dir = Path::new(&output_dir);
    let template_output_dir = output_dir.join("templates");

    let templates_dir = "templates";
    println!("cargo:rerun-if-changed={templates_dir}/");
    let templates_dir = Path::new(templates_dir);

    std::fs::create_dir_all(&template_output_dir).unwrap();
    for t in templates_dir.read_dir().unwrap() {
        let template_path = t.unwrap().path();
        if !matches!(
            template_path.extension().map(|s| s.to_str().unwrap()),
            Some("html") | Some("css")
        ) {
            continue;
        }
        minify_template(&template_path, &template_output_dir);
    }
}
