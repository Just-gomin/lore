//! `lore-extractor-image`: 이미지의 글자를 외부 OCR 도구 tesseract로 읽어 소스 노트 후보로 바꾸는 추출기.
//!
//! 그림의 뜻(도표, 화살표)은 읽지 않습니다. 스킬이 인식한 글과 그림을 함께 보고 정제합니다.
//! 입력과 출력은 기획서 6장의 추출기 규약을 따릅니다.

use std::io::{Cursor, ErrorKind};
use std::path::Path;
use std::process::{Command, ExitCode, Output};

use exif::{In, Tag, Value};
use lore_connector_common::ConnectorError;
use lore_connector_common::extractor::{self, Extracted, is_date};

/// 입력에 `lang`이 없을 때 읽는 언어. tesseract의 언어 이름을 `+`로 잇습니다.
const DEFAULT_LANG: &str = "kor+eng";

const INSTALL: &str = "tesseract를 설치하세요(macOS: brew install tesseract, Ubuntu: sudo apt-get install tesseract-ocr). 다른 위치의 실행 파일은 LORE_TESSERACT 환경 변수로 지정합니다.";
const INSTALL_LANG: &str = "언어 데이터가 없으면 설치하세요(macOS: brew install tesseract-lang, Ubuntu: sudo apt-get install tesseract-ocr-kor).";

fn main() -> ExitCode {
    extractor::run(|input, extra, bytes| {
        let lang = extra
            .string("lang")?
            .unwrap_or_else(|| DEFAULT_LANG.to_string());
        if lang.is_empty()
            || !lang
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+')
        {
            return Err(ConnectorError::new(
                "invalid_input",
                format!(
                    "lang '{lang}'는 kor+eng처럼 tesseract 언어 이름을 +로 이은 것이어야 합니다."
                ),
            ));
        }
        if !is_image(bytes) {
            return Err(ConnectorError::new(
                "unreadable_file",
                "PNG나 JPEG 이미지가 아닙니다.",
            ));
        }
        let tool = std::env::var_os("LORE_TESSERACT").unwrap_or_else(|| "tesseract".into());
        let tool = Path::new(&tool);
        let version = version(tool)?;
        let text = recognize(tool, &input.file, &lang)?;
        Ok(Extracted {
            title: None,
            occurred_at: exif_date(bytes),
            text: describe(&version, &lang, &text),
        })
    })
}

/// PNG와 JPEG만 받습니다. HEIC 같은 형식은 tesseract가 읽지 못합니다.
fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xff, 0xd8, 0xff])
}

fn run(tool: &Path, args: &[&std::ffi::OsStr]) -> Result<Output, ConnectorError> {
    Command::new(tool).args(args).output().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            ConnectorError::new(
                "tool_missing",
                format!("{}를 찾지 못했습니다. {INSTALL}", tool.display()),
            )
        } else {
            ConnectorError::new(
                "tool_failed",
                format!("{}를 실행하지 못했습니다: {error}", tool.display()),
            )
        }
    })
}

/// `tesseract --version`의 첫 줄. 4.x는 표준 오류로 내므로 둘 다 봅니다.
fn version(tool: &Path) -> Result<String, ConnectorError> {
    let output = run(tool, &["--version".as_ref()])?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("tesseract")
        .to_string())
}

fn recognize(tool: &Path, file: &Path, lang: &str) -> Result<String, ConnectorError> {
    let output = run(
        tool,
        &[
            file.as_os_str(),
            "stdout".as_ref(),
            "-l".as_ref(),
            lang.as_ref(),
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ConnectorError::new(
            "tool_failed",
            format!(
                "tesseract가 실패했습니다: {}. {INSTALL_LANG}",
                stderr.trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 어떤 도구로 읽었는지 한 줄을 맨 위에 두고, 인식한 글을 붙입니다.
fn describe(version: &str, lang: &str, text: &str) -> String {
    // tesseract는 쪽 끝에 폼 피드를 냅니다.
    let text = text.replace('\u{c}', "");
    let text = text.trim();
    let body = if text.is_empty() {
        "인식한 글자가 없습니다."
    } else {
        text
    };
    format!("(글자 인식: {version}, 언어 {lang}. 인식 오류가 있을 수 있습니다.)\n\n{body}")
}

/// EXIF 촬영 시각(`DateTimeOriginal`, 없으면 `DateTime`)의 날짜 부분. 시간대가 없는 값이라 적힌 날짜 그대로입니다.
fn exif_date(bytes: &[u8]) -> Option<String> {
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()?;
    [Tag::DateTimeOriginal, Tag::DateTime]
        .into_iter()
        .find_map(|tag| {
            let field = exif.get_field(tag, In::PRIMARY)?;
            let Value::Ascii(values) = &field.value else {
                return None;
            };
            let text = String::from_utf8_lossy(values.first()?);
            let date = text.get(..10)?.replace(':', "-");
            is_date(&date).then_some(date)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 인식한_글이_없으면_없다고_적는다() {
        assert_eq!(
            describe("tesseract 5.3.4", "eng", " \n\u{c}"),
            "(글자 인식: tesseract 5.3.4, 언어 eng. 인식 오류가 있을 수 있습니다.)\n\n인식한 글자가 없습니다."
        );
    }

    #[test]
    fn png와_jpeg만_받는다() {
        assert!(is_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(is_image(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(!is_image(b"\0\0\0\x18ftypheic"));
    }
}
