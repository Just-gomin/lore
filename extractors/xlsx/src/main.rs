//! `lore-extractor-xlsx`: 엑셀과 OpenDocument 스프레드시트를 소스 노트 후보로 바꾸는 추출기.
//!
//! 보이는 시트마다 `## <시트 이름>` 아래 마크다운 표로 냅니다. 서식은 적용하지 않고, 수식은 파일에 저장된
//! 계산값을 씁니다. 입력과 출력은 기획서 6장의 추출기 규약을 따릅니다.

use std::io::Cursor;
use std::process::ExitCode;

use calamine::{Data, Reader, SheetType, SheetVisible, open_workbook_auto_from_rs};
use chrono::{NaiveDateTime, Timelike};
use lore_connector_common::ConnectorError;
use lore_connector_common::extractor::{self, Extracted, local_date};

/// 시트마다 내는 데이터 행 수의 기본 상한. 머리글 행은 세지 않습니다.
const MAX_ROWS: usize = 500;

fn main() -> ExitCode {
    extractor::run(|_, extra, bytes| {
        let max_rows = extra.count("max_rows")?.unwrap_or(MAX_ROWS);
        extract(bytes, max_rows)
    })
}

fn unreadable(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError::new(
        "unreadable_file",
        format!("스프레드시트로 읽지 못했습니다: {error}"),
    )
}

fn extract(bytes: &[u8], max_rows: usize) -> Result<Extracted, ConnectorError> {
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(unreadable)?;
    let properties = workbook.workbook_properties().map_err(unreadable)?;

    let sheets = workbook.sheets_metadata().to_vec();
    let mut blocks = Vec::new();
    let mut hidden = 0;
    for sheet in sheets {
        if sheet.typ != SheetType::WorkSheet {
            continue;
        }
        if sheet.visible != SheetVisible::Visible {
            hidden += 1;
            continue;
        }
        let range = workbook.worksheet_range(&sheet.name).map_err(unreadable)?;
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(cell).collect())
            .collect();
        blocks.push(format!("## {}\n\n{}", sheet.name, table(rows, max_rows)));
    }
    if hidden > 0 {
        blocks.push(format!("(숨긴 시트 {hidden}개는 넣지 않았습니다.)"));
    }

    Ok(Extracted {
        title: properties.title.map(|title| title.trim().to_string()),
        occurred_at: properties.modified.as_deref().and_then(local_date),
        text: blocks.join("\n\n"),
    })
}

/// 칸 하나를 글로 씁니다. 서식은 적용하지 않습니다.
fn cell(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        Data::String(text) => text.clone(),
        Data::Int(number) => number.to_string(),
        Data::Float(number) => float(*number),
        Data::Bool(value) => if *value { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(time) if time.is_duration() => match time.as_duration() {
            Some(duration) => {
                let seconds = duration.num_seconds();
                format!(
                    "{}:{:02}:{:02}",
                    seconds / 3600,
                    (seconds % 3600) / 60,
                    seconds % 60
                )
            }
            None => float(time.as_f64()),
        },
        Data::DateTime(time) => match time.as_datetime() {
            Some(time) => date_time(time),
            None => float(time.as_f64()),
        },
        Data::DateTimeIso(text) => {
            match NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f") {
                Ok(time) => date_time(time),
                Err(_) => text.clone(),
            }
        }
        Data::DurationIso(text) => text.clone(),
        Data::Error(error) => error.to_string(),
    }
}

/// 정수면 소수점 없이, 아니면 Rust `f64`의 기본 표현입니다.
fn float(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

/// `YYYY-MM-DD`, 시각이 있으면 `YYYY-MM-DD HH:MM`입니다.
fn date_time(time: NaiveDateTime) -> String {
    if time.hour() == 0 && time.minute() == 0 && time.second() == 0 {
        time.format("%Y-%m-%d").to_string()
    } else {
        time.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// 행들을 마크다운 표로 씁니다. 첫 행이 머리글이고, 모든 칸이 빈 행과 끝의 빈 열은 뺍니다.
fn table(rows: Vec<Vec<String>>, max_rows: usize) -> String {
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        .collect();
    let width = rows
        .iter()
        .filter_map(|row| row.iter().rposition(|cell| !cell.trim().is_empty()))
        .max()
        .map_or(0, |last| last + 1);
    if rows.is_empty() || width == 0 {
        return "(빈 시트)".to_string();
    }
    let line = |row: &[String]| {
        let cells: Vec<String> = (0..width)
            .map(|index| escape(row.get(index).map_or("", String::as_str)))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = vec![line(&rows[0]), format!("|{}", " --- |".repeat(width))];
    let body = &rows[1..];
    lines.extend(body.iter().take(max_rows).map(|row| line(row)));
    let mut text = lines.join("\n");
    if body.len() > max_rows {
        text.push_str(&format!(
            "\n\n(남은 행 {}개는 넣지 않았습니다.)",
            body.len() - max_rows
        ));
    }
    text
}

/// 표가 깨지지 않게 `|`와 줄바꿈을 바꿉니다.
fn escape(text: &str) -> String {
    text.trim()
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace(['\n', '\r'], "<br>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|cell| cell.to_string()).collect())
            .collect()
    }

    #[test]
    fn 빈_행과_끝의_빈_열을_빼고_표로_쓴다() {
        let text = table(
            rows(&[&["항목", "값", ""], &["", "", ""], &["a|b", "줄\n바꿈", ""]]),
            10,
        );
        assert_eq!(text, "| 항목 | 값 |\n| --- | --- |\n| a\\|b | 줄<br>바꿈 |");
        assert_eq!(table(rows(&[&["", ""]]), 10), "(빈 시트)");
    }

    #[test]
    fn 상한을_넘는_행은_잘라_내고_남은_수를_적는다() {
        let text = table(rows(&[&["n"], &["1"], &["2"], &["3"]]), 2);
        assert_eq!(
            text,
            "| n |\n| --- |\n| 1 |\n| 2 |\n\n(남은 행 1개는 넣지 않았습니다.)"
        );
    }

    #[test]
    fn 수는_정수면_소수점_없이_쓴다() {
        assert_eq!(float(3.0), "3");
        assert_eq!(float(-12.0), "-12");
        assert_eq!(float(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float(1.5), "1.5");
        assert_eq!(float(1e20), "100000000000000000000");
    }
}
