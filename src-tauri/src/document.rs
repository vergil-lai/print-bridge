use crate::protocol::EffectivePaper;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgb};
use printpdf::{
    Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Pt, RawImage, RawImageData, RawImageFormat,
    XObjectTransform,
};
use std::{fs, io::Read, path::Path};
use thiserror::Error;

const HEADER_SIZE: usize = 8;
const DEFAULT_DPI: f32 = 203.0;

/// Agent 在打印前可以校验或规范化的文档格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Pdf,
    Png,
    Jpeg,
}

/// 用于把图片放入目标纸张尺寸内的矩形区域。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 检测或规范化可打印文档时返回的错误。
#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("unsupported document format")]
    UnsupportedFormat,
    #[error("invalid image dimensions")]
    InvalidImageDimensions,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
}

pub type DocumentResult<T> = Result<T, DocumentError>;

/// 根据文件开头字节检测文档格式。
pub fn detect_format_from_bytes(bytes: &[u8]) -> Option<DocumentFormat> {
    if bytes.starts_with(b"%PDF-") {
        Some(DocumentFormat::Pdf)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(DocumentFormat::Png)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(DocumentFormat::Jpeg)
    } else {
        None
    }
}

/// 只读取文件签名头来检测文件格式。
pub fn detect_format(path: &Path) -> DocumentResult<Option<DocumentFormat>> {
    let mut header = [0_u8; HEADER_SIZE];
    let bytes_read = fs::File::open(path)?.read(&mut header)?;

    Ok(detect_format_from_bytes(&header[..bytes_read]))
}

/// 计算图片在页面中居中且完整包含的排版矩形。
pub fn fit_contain(
    image_width: f64,
    image_height: f64,
    page_width: f64,
    page_height: f64,
) -> DocumentResult<FitRect> {
    if !is_positive_finite(image_width)
        || !is_positive_finite(image_height)
        || !is_positive_finite(page_width)
        || !is_positive_finite(page_height)
    {
        return Err(DocumentError::InvalidImageDimensions);
    }

    let scale = (page_width / image_width).min(page_height / image_height);
    let width = image_width * scale;
    let height = image_height * scale;

    Ok(FitRect {
        x: (page_width - width) / 2.0,
        y: (page_height - height) / 2.0,
        width,
        height,
    })
}

/// 把 PNG 或 JPEG 转成匹配目标纸张的一页 PDF。
pub fn image_to_pdf(
    image_path: &Path,
    paper: &EffectivePaper,
    output_path: &Path,
) -> DocumentResult<()> {
    match detect_format(image_path)? {
        Some(DocumentFormat::Png | DocumentFormat::Jpeg) => {}
        _ => return Err(DocumentError::UnsupportedFormat),
    }

    paper
        .validate()
        .map_err(|_| DocumentError::InvalidImageDimensions)?;

    let image = image::open(image_path)?;
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return Err(DocumentError::InvalidImageDimensions);
    }

    let fit = fit_contain(
        image_width as f64,
        image_height as f64,
        paper.width_mm,
        paper.height_mm,
    )?;
    let raw_image = raw_image_from_dynamic_image(image);

    // PDF 打印坐标使用物理单位，所以先把图片像素映射到稳定的
    // 标签打印机 DPI，再缩放进计算好的排版矩形。
    let mut doc = PdfDocument::new("print-bridge-document");
    let image_id = doc.add_image(&raw_image);
    let natural_width_mm = image_width as f64 * 25.4 / f64::from(DEFAULT_DPI);
    let natural_height_mm = image_height as f64 * 25.4 / f64::from(DEFAULT_DPI);

    let page = PdfPage::new(
        Mm(paper.width_mm as f32),
        Mm(paper.height_mm as f32),
        vec![Op::UseXobject {
            id: image_id,
            transform: XObjectTransform {
                translate_x: Some(mm_to_pt(fit.x)),
                translate_y: Some(mm_to_pt(fit.y)),
                rotate: None,
                scale_x: Some((fit.width / natural_width_mm) as f32),
                scale_y: Some((fit.height / natural_height_mm) as f32),
                dpi: Some(DEFAULT_DPI),
            },
        }],
    );

    let bytes = doc
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    fs::write(output_path, bytes)?;

    Ok(())
}

/// 把毫米转换为 printpdf 变换所需的 PDF 点。
fn mm_to_pt(value: f64) -> Pt {
    Mm(value as f32).into()
}

/// 把图片转换为 printpdf 需要的 RGB 原始图片格式。
fn raw_image_from_dynamic_image(image: DynamicImage) -> RawImage {
    let (width, height, pixels) = flatten_to_white(image);

    RawImage {
        pixels: RawImageData::U8(pixels),
        width: width as usize,
        height: height as usize,
        data_format: RawImageFormat::RGB8,
        tag: Vec::new(),
    }
}

/// 把透明度合成到白底上，保证透明标签打印结果可预期。
fn flatten_to_white(image: DynamicImage) -> (u32, u32, Vec<u8>) {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = ImageBuffer::new(width, height);

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = f32::from(pixel[3]) / 255.0;
        let red = blend_channel(pixel[0], alpha);
        let green = blend_channel(pixel[1], alpha);
        let blue = blend_channel(pixel[2], alpha);
        rgb.put_pixel(x, y, Rgb([red, green, blue]));
    }

    (width, height, rgb.into_raw())
}

/// 把单个颜色通道与白色背景混合。
fn blend_channel(channel: u8, alpha: f32) -> u8 {
    (f32::from(channel) * alpha + 255.0 * (1.0 - alpha)).round() as u8
}

/// 检查尺寸是否能安全参与布局计算。
fn is_positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
