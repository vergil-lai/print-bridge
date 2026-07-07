use crate::protocol::SupportedFormat;
use office2pdf::config::{ConvertOptions, Format};
use std::{fs, io, path::Path};
use thiserror::Error;
use zip::{result::ZipError, ZipArchive};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeFormat {
    Docx,
    Xlsx,
    Pptx,
}

#[derive(Debug, Error)]
pub enum OfficeConvertError {
    #[error("unsupported office format")]
    UnsupportedFormat,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("office conversion failed: {0}")]
    Convert(#[from] office2pdf::error::ConvertError),
}

pub fn office_format_from_supported(format: SupportedFormat) -> Option<OfficeFormat> {
    match format {
        SupportedFormat::Docx => Some(OfficeFormat::Docx),
        SupportedFormat::Xlsx => Some(OfficeFormat::Xlsx),
        SupportedFormat::Pptx => Some(OfficeFormat::Pptx),
        _ => None,
    }
}

pub fn detect_office_format(path: &Path) -> Result<Option<OfficeFormat>, OfficeConvertError> {
    let file = fs::File::open(path)?;
    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(ZipError::InvalidArchive(_)) => return Ok(None),
        Err(ZipError::UnsupportedArchive(_)) => return Ok(None),
        Err(ZipError::Io(error)) => return Err(OfficeConvertError::Io(error)),
        Err(_) => return Ok(None),
    };

    if archive.by_name("word/document.xml").is_ok() {
        return Ok(Some(OfficeFormat::Docx));
    }
    if archive.by_name("xl/workbook.xml").is_ok() {
        return Ok(Some(OfficeFormat::Xlsx));
    }
    if archive.by_name("ppt/presentation.xml").is_ok() {
        return Ok(Some(OfficeFormat::Pptx));
    }

    Ok(None)
}

pub fn office_to_pdf(
    input_path: &Path,
    format: OfficeFormat,
    output_path: &Path,
) -> Result<(), OfficeConvertError> {
    let data = fs::read(input_path)?;
    let result =
        office2pdf::convert_bytes(&data, office2pdf_format(format), &ConvertOptions::default())?;
    fs::write(output_path, result.pdf)?;
    Ok(())
}

fn office2pdf_format(format: OfficeFormat) -> Format {
    match format {
        OfficeFormat::Docx => Format::Docx,
        OfficeFormat::Xlsx => Format::Xlsx,
        OfficeFormat::Pptx => Format::Pptx,
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_office_format, office_format_from_supported, OfficeFormat};
    use crate::protocol::SupportedFormat;
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn maps_supported_formats_to_office_formats() {
        assert_eq!(
            office_format_from_supported(SupportedFormat::Docx),
            Some(OfficeFormat::Docx)
        );
        assert_eq!(
            office_format_from_supported(SupportedFormat::Xlsx),
            Some(OfficeFormat::Xlsx)
        );
        assert_eq!(
            office_format_from_supported(SupportedFormat::Pptx),
            Some(OfficeFormat::Pptx)
        );
        assert_eq!(office_format_from_supported(SupportedFormat::Pdf), None);
    }

    #[test]
    fn detects_minimal_ooxml_containers() {
        let docx = temp_path("sample.docx");
        let xlsx = temp_path("sample.xlsx");
        let pptx = temp_path("sample.pptx");
        let _ = fs::remove_file(&docx);
        let _ = fs::remove_file(&xlsx);
        let _ = fs::remove_file(&pptx);

        write_zip(&docx, &["word/document.xml"]);
        write_zip(&xlsx, &["xl/workbook.xml"]);
        write_zip(&pptx, &["ppt/presentation.xml"]);

        assert_eq!(
            detect_office_format(&docx).unwrap(),
            Some(OfficeFormat::Docx)
        );
        assert_eq!(
            detect_office_format(&xlsx).unwrap(),
            Some(OfficeFormat::Xlsx)
        );
        assert_eq!(
            detect_office_format(&pptx).unwrap(),
            Some(OfficeFormat::Pptx)
        );

        let _ = fs::remove_file(&docx);
        let _ = fs::remove_file(&xlsx);
        let _ = fs::remove_file(&pptx);
    }

    fn write_zip(path: &Path, entries: &[&str]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for entry in entries {
            zip.start_file(entry, options).unwrap();
            zip.write_all(b"<xml/>").unwrap();
        }
        zip.finish().unwrap();
    }

    fn temp_path(file_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "print-bridge-office-test-{}-{file_name}",
            std::process::id()
        ))
    }
}
