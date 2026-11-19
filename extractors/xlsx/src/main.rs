//! `lore-extractor-xlsx`: 엑셀과 OpenDocument 스프레드시트를 소스 노트 후보로 바꾸는 추출기.
//!
//! 보이는 시트마다 `## <시트 이름>` 아래 마크다운 표로 냅니다. 서식은 적용하지 않고, 수식은 파일에 저장된
//! 계산값을 씁니다. 입력과 출력은 기획서 6장의 추출기 규약을 따릅니다.

use std::io::Cursor;
use std::process::ExitCode;

use calamine::{Data, Reader, SheetType, SheetVisible, open_workbook_auto_from_rs};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Timelike};
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
    // 문서 속성이 깨졌어도 표는 읽습니다. 제목과 날짜는 입력으로 받을 수 있습니다.
    let properties = workbook.workbook_properties().unwrap_or_default();

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
        // 시트 하나를 읽지 못해도(calamine이 모르는 새 오류 값 `#SPILL!` 등) 나머지 시트는 냅니다.
        let content = match workbook.worksheet_range(&sheet.name) {
            Ok(range) => {
                let rows: Vec<Vec<String>> = range
                    .rows()
                    .map(|row| row.iter().map(cell).collect())
                    .collect();
                table(rows, max_rows)
            }
            Err(error) => format!("(이 시트는 읽지 못했습니다: {error})"),
        };
        blocks.push(format!("## {}\n\n{content}", sheet.name));
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
        Data::DateTime(time) => {
            let value = time.as_f64();
            if time.is_duration() {
                // 범위를 넘는 값은 calamine이 시간으로 바꾸다 멈추므로 수로 씁니다.
                if value.is_finite() && value.abs() < 1e7 {
                    duration((value * 86400.0).round() as i64)
                } else {
                    float(value)
                }
            } else if (0.0..1.0).contains(&value) {
                // 시각만 든 칸(`h:mm` 서식)은 날짜 없이 `H:MM:SS`로 씁니다. ods는 시각과 기간을 같은 꼴로
                // 저장해 둘을 가를 수 없으므로, 두 형식이 같은 글을 내도록 기간과 같게 씁니다.
                duration((value * 86400.0).round() as i64)
            } else if (1.0..2_958_466.0).contains(&value)
                && let Some(time) = time.as_datetime()
            {
                date_time(time)
            } else {
                float(value)
            }
        }
        Data::DateTimeIso(text) => iso_date_time(text),
        Data::DurationIso(text) => iso_duration(text).map_or_else(|| text.clone(), duration),
        Data::Error(error) => error.to_string(),
    }
}

/// 초를 `H:MM:SS`로 씁니다. 음수면 앞에 `-`를 붙입니다.
fn duration(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let seconds = seconds.unsigned_abs();
    format!(
        "{sign}{}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// ISO 8601 날짜와 시각. 시간대가 있어도 적힌 시각 그대로 써서 어느 컴퓨터에서나 같게 냅니다.
fn iso_date_time(text: &str) -> String {
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return date_time(time.naive_local());
    }
    if let Ok(time) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f") {
        return date_time(time);
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return date.format("%Y-%m-%d").to_string();
    }
    text.to_string()
}

/// ISO 8601 기간(`PT10H30M00S`, `-P1DT2H`)을 초로 읽습니다. ODS의 시각과 기간 칸이 이 꼴입니다.
fn iso_duration(text: &str) -> Option<i64> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let rest = rest.strip_prefix('P')?;
    let (days, time) = match rest.split_once('T') {
        Some((days, time)) => (days, time),
        None => (rest, ""),
    };
    let mut total = 0.0;
    if !days.is_empty() {
        total += days.strip_suffix('D')?.parse::<f64>().ok()? * 86400.0;
    }
    let mut number = String::new();
    for c in time.chars() {
        let unit = match c {
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => {
                number.push(c);
                continue;
            }
        };
        total += number.parse::<f64>().ok()? * unit;
        number.clear();
    }
    if !number.is_empty() || !total.is_finite() {
        return None;
    }
    let seconds = total.round() as i64;
    Some(if negative { -seconds } else { seconds })
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

/// 표가 깨지지 않게 `\`, `|`, 줄바꿈을 바꿉니다. `\`를 먼저 바꿔야 `a\|b`가 칸을 나누지 않습니다.
fn escape(text: &str) -> String {
    text.trim()
        .replace('\\', "\\\\")
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
    fn 기간과_시각과_iso_값을_같은_꼴로_쓴다() {
        assert_eq!(duration(-5400), "-1:30:00");
        assert_eq!(duration(37800), "10:30:00");
        assert_eq!(iso_duration("PT10H30M00S"), Some(37800));
        assert_eq!(iso_duration("PT36H00M00S"), Some(129600));
        assert_eq!(iso_duration("-P1DT2H"), Some(-93600));
        assert_eq!(iso_duration("PT1.5S"), Some(2));
        assert_eq!(iso_duration("10:30"), None);
        assert_eq!(
            iso_date_time("2026-11-03T10:30:00+09:00"),
            "2026-11-03 10:30"
        );
        assert_eq!(iso_date_time("2026-11-03T10:30:00Z"), "2026-11-03 10:30");
        assert_eq!(iso_date_time("2026-11-03"), "2026-11-03");
        assert_eq!(iso_date_time("모름"), "모름");
        assert_eq!(escape("c\\|d"), "c\\\\\\|d");
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
