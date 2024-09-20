use axum::{
    body::Body,
    extract::{self, Query, State},
    http::{HeaderMap, Response, StatusCode},
};
use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{ensure, ContextCompat, WrapErr},
    Result,
};
use std::{collections::HashMap, io::SeekFrom, path::PathBuf};
use tokio::io::{AsyncSeekExt as _, BufReader};
use tracing::{debug, info};

type Ranges = Vec<(Option<u64>, Option<u64>)>;

use crate::utils::content_type_from_extension;
use crate::AppState;

pub async fn dl_range(
    path_relative_to_data: &Utf8Path,
    file_name: &str,
    file_len: u64,
    ranges: Ranges,
    content_type: &str,
) -> Result<Response<Body>, (StatusCode, String)> {
    debug!("User made a range request");
    debug!(?ranges);

    let start = if ranges.is_empty() {
        return Err((StatusCode::RANGE_NOT_SATISFIABLE, "You shouldn't send a range request without an actual range. That's bad for the environment".to_string()));
    } else if ranges.len() > 1 {
        return Err((
            StatusCode::RANGE_NOT_SATISFIABLE,
            "Do not support multiple ranges in Range request".to_string(),
        ));
    } else {
        let start = ranges[0]
            .0
            .context("Range without starting range not supported")
            .map_err(|e| (StatusCode::RANGE_NOT_SATISFIABLE, e.to_string()))?;

        if start > file_len {
            Err((
                StatusCode::RANGE_NOT_SATISFIABLE,
                "The range start was past the end of the file".to_string(),
            ))
        } else {
            Ok(start)
        }
    }?;

    let mut file = tokio::fs::File::open(&path_relative_to_data)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let start_i64 = start.try_into().map_err(|_| {
        (
            StatusCode::RANGE_NOT_SATISFIABLE,
            "Range value too big to fit into i64 (9 exabytes)".to_string(),
        )
    })?;
    file.seek(SeekFrom::Current(start_i64)).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to seek file to {start} bytes: {e}"),
        )
    })?;

    let buffered_file = BufReader::new(file);
    let stream = tokio_util::io::ReaderStream::new(buffered_file);
    let stream = axum::body::Body::from_stream(stream);
    let sent_len = file_len - start;
    let end = file_len - 1;
    // FIXME: Transfer-Encoding maybe?
    Response::builder()
        .status(206)
        .header("Content-Range", format!("bytes {start}-{end}/{file_len}"))
        .header("Content-Length", sent_len)
        .header("Content-Type", content_type)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{file_name}\""),
        )
        .body(stream)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn dl_path(
    extract::Path(fetched_path): extract::Path<PathBuf>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response<Body>, (StatusCode, String)> {
    let fetched_path = Utf8PathBuf::from_path_buf(fetched_path)
        .map_err(|p| (StatusCode::BAD_REQUEST, format!("Path {p:?} was not UTF-8")))?;
    info!(?fetched_path, "Downloading path");

    let path_relative_to_data = {
        let data_dir = &state.data_dir;
        let mut p = data_dir.to_path_buf();
        p.push(&fetched_path);
        p
    };

    let metadata = {
        let metadata = tokio::fs::metadata(&path_relative_to_data)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

        if metadata.is_dir() {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("TODO: Cannot download folders yet: requested {fetched_path:?}"),
            ));
        }
        metadata
    };

    let file_len = metadata.len();
    let ext = fetched_path.extension();
    let content_type = content_type_from_extension(ext);
    let file_name = path_relative_to_data
        .file_name()
        .expect("File name should be some since it is validated");

    if let Some(ranges) = headers.get("Range") {
        let ranges = ranges
            .to_str()
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let ranges = parse_ranges(ranges).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        dl_range(
            &path_relative_to_data,
            file_name,
            file_len,
            ranges,
            content_type,
        )
        .await
    } else {
        let file = tokio::fs::File::open(&path_relative_to_data)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
        let buffered_file = BufReader::new(file);
        let stream = tokio_util::io::ReaderStream::new(buffered_file);
        let stream = axum::body::Body::from_stream(stream);

        Response::builder()
            .status(200)
            .header("Accept-Ranges", "bytes")
            .header("Content-Length", file_len)
            .header("Content-Type", content_type)
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{file_name}\""),
            )
            .body(stream)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    }
}

pub fn parse_ranges(range: &str) -> Result<Ranges> {
    // bytes = <num1>-<num2>,<num3>-<num4>
    let range_str = {
        let mut vals = range.split('=');

        ensure!(
            vals.next().context("Missing range unit")? == "bytes",
            "Range unit was not bytes"
        );

        let range_str = vals.next().context("Missing range values")?;
        ensure!(
            vals.next().is_none(),
            "Range format should be `bytes=<num>-<num>,...`, was {range}"
        );
        range_str
    };

    let mut res = vec![];
    for range in range_str.split(',') {
        let mut it = range.split('-');
        let v1 = it.next().wrap_err("Missing any value for range")?;
        let v1 = if v1.is_empty() {
            None
        } else {
            Some(
                v1.parse::<u64>()
                    .wrap_err_with(|| format!("Invalid value for range: {v1}"))?,
            )
        };

        let v2 = it.next().wrap_err("Missing `-` in range")?;
        let v2 = if v2.is_empty() {
            None
        } else {
            Some(
                v2.parse::<u64>()
                    .wrap_err_with(|| format!("Invalid value for range: {v2}"))?,
            )
        };
        ensure!(
            it.next().is_none(),
            "Range {range} should be in format <num1>-<num2>"
        );
        res.push((v1, v2));
    }
    Ok(res)
}

pub async fn dl_archive(
    extract::Path(fetched_path): extract::Path<PathBuf>,
    State(_): State<AppState>,
    _: HeaderMap,
    Query(query): Query<HashMap<String, Option<Vec<String>>>>,
) -> Result<Response<Body>, String> {
    info!(?fetched_path, ?query, "Downloading archive from path");
    todo!()
}
