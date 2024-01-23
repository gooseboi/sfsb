use itertools::{EitherOrBoth::{Left, Right, Both}, Itertools as _};
use std::cmp::Ordering;

pub fn content_type_from_extension(ext: Option<&str>) -> &str {
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Basics_of_HTTP/MIME_types/Common_types
    let Some(ext) = ext else {
        // FIXME: text/plain or application/octet-stream?
        return "text/plain";
    };
    #[allow(clippy::wildcard_in_or_patterns)]
    match ext {
        ".aac" => "audio/aac",
        ".abw" => "application/x-abiword",
        ".apng" => "image/apng",
        ".arc" => "application/x-freearc",
        ".avif" => "image/avif",
        ".avi" => "video/x-msvideo",
        ".azw" => "application/vnd.amazon.ebook",
        ".bin" => "application/octet-stream",
        ".bmp" => "image/bmp",
        ".bz" => "application/x-bzip",
        ".bz2" => "application/x-bzip2",
        ".cda" => "application/x-cdf",
        ".csh" => "application/x-csh",
        ".css" => "text/css",
        ".csv" => "text/csv",
        ".doc" => "application/msword",
        ".docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".eot" => "application/vnd.ms-fontobject",
        ".epub" => "application/epub+zip",
        ".gz" => "application/gzip",
        ".gif" => "image/gif",
        ".htm" | ".html" => "text/html",
        ".ico" => "image/vnd.microsoft.icon",
        ".ics" => "text/calendar",
        ".jar" => "application/java-archive",
        ".jpeg" | ".jpg" => "image/jpeg",
        ".mjs" | ".js" => "text/javascript",
        ".json" => "application/json",
        ".jsonld" => "application/ld+json",
        ".mid," => "audio/midi",
        ".mp3" => "audio/mpeg",
        ".mp4" => "video/mp4",
        ".mpeg" => "video/mpeg",
        ".mpkg" => "application/vnd.apple.installer+xml",
        ".odp" => "application/vnd.oasis.opendocument.presentation",
        ".ods" => "application/vnd.oasis.opendocument.spreadsheet",
        ".odt" => "application/vnd.oasis.opendocument.text",
        ".oga" => "audio/ogg",
        ".ogv" => "video/ogg",
        ".ogx" => "application/ogg",
        ".opus" => "audio/opus",
        ".otf" => "font/otf",
        ".png" => "image/png",
        ".pdf" => "application/pdf",
        ".php" => "application/x-httpd-php",
        ".ppt" => "application/vnd.ms-powerpoint",
        ".pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ".rar" => "application/vnd.rar",
        ".rtf" => "application/rtf",
        ".sh" => "application/x-sh",
        ".svg" => "image/svg+xml",
        ".tar" => "application/x-tar",
        ".tif" | ".tiff" => "image/tiff",
        ".ts" => "video/mp2t",
        ".ttf" => "font/ttf",
        ".vsd" => "application/vnd.visio",
        ".wav" => "audio/wav",
        ".weba" => "audio/webm",
        ".webm" => "video/webm",
        ".webp" => "image/webp",
        ".woff" => "font/woff",
        ".woff2" => "font/woff2",
        ".xhtml" => "application/xhtml+xml",
        ".xls" => "application/vnd.ms-excel",
        ".xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ".xml" => "application/xml",
        ".xul" => "application/vnd.mozilla.xul+xml",
        ".zip" => "application/zip",
        ".3gp" => "video/3gpp",
        ".3g2" => "video/3gpp2",
        ".7z" => "application/x-7z-compressed",
        // FIXME: Same as above
        ".txt" | _ => "text/plain",
    }
}

pub fn cmp_ignore_case_utf8(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .zip_longest(b.chars().flat_map(char::to_lowercase))
        .map(|ab| match ab {
            Left(_) => Ordering::Greater,
            Right(_) => Ordering::Less,
            Both(a, b) => a.cmp(&b),
        })
        .find(|&ordering| ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}
