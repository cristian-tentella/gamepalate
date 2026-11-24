use dav1d::pixel::{MatrixCoefficients, YUVRange};
use dav1d::{Decoder, Error as Av1Error, Picture, PixelLayout, PlanarImageComponent, Settings};
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

const MAX_ATTEMPTS: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum AvifError {
    #[error("the AVIF container is not valid")]
    Container(#[from] avif_parse::Error),

    #[error("the AV1 decoder failed: {0:?}")]
    Decoder(Av1Error),

    #[error("the colors could not be converted")]
    Colors(#[from] yuv::YuvError),

    #[error("{0} is not supported")]
    Unsupported(&'static str),
}

pub fn decode(mut bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), AvifError> {
    let avif = avif_parse::read_avif(&mut bytes)?;
    let color = picture(&avif.primary_item)?;
    let (width, height) = (color.width(), color.height());
    let mut rgba = vec![255; width as usize * height as usize * 4];
    to_rgba(&color, &mut rgba)?;

    if let Some(alpha) = avif.alpha_item.as_deref() {
        let alpha = picture(alpha)?;
        if (alpha.width(), alpha.height()) != (width, height) {
            return Err(AvifError::Unsupported("an alpha plane of another size"));
        }
        let full = alpha.color_range() == YUVRange::Full;
        let plane = plane_8bit(&alpha, PlanarImageComponent::Y);
        for (pixel, &value) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(&plane) {
            pixel[3] = luma(value, full);
        }
        if avif.premultiplied_alpha {
            unpremultiply(&mut rgba);
        }
    }

    Ok((width, height, rgba))
}

fn picture(data: &[u8]) -> Result<Picture, AvifError> {
    let mut settings = Settings::new();
    settings.set_max_frame_delay(1);
    let mut decoder = Decoder::with_settings(&settings).map_err(AvifError::Decoder)?;
    match decoder.send_data(data.to_vec(), None, None, None) {
        Ok(()) | Err(Av1Error::Again) => {}
        Err(error) => return Err(AvifError::Decoder(error)),
    }
    for _ in 0..MAX_ATTEMPTS {
        match decoder.get_picture() {
            Ok(picture) => return Ok(picture),
            Err(Av1Error::Again) => match decoder.send_pending_data() {
                Ok(()) | Err(Av1Error::Again) => std::thread::yield_now(),
                Err(error) => return Err(AvifError::Decoder(error)),
            },
            Err(error) => return Err(AvifError::Decoder(error)),
        }
    }

    Err(AvifError::Unsupported("an image without a frame"))
}

fn plane_8bit(picture: &Picture, component: PlanarImageComponent) -> Vec<u8> {
    let (width, height) = plane_size(picture, component);
    let stride = picture.stride(component) as usize;
    let plane = picture.plane(component);
    let bytes: &[u8] = plane.as_ref();
    let depth = picture.bit_depth();
    let mut tight = Vec::with_capacity(width * height);
    for row in bytes.chunks(stride).take(height) {
        match depth {
            8 => tight.extend_from_slice(&row[..width]),
            _ => tight.extend(
                row[..width * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|sample| (u16::from_ne_bytes(*sample) >> (depth - 8)) as u8),
            ),
        }
    }

    tight
}

fn plane_size(picture: &Picture, component: PlanarImageComponent) -> (usize, usize) {
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    match (component, picture.pixel_layout()) {
        (PlanarImageComponent::Y, _) | (_, PixelLayout::I444 | PixelLayout::I400) => {
            (width, height)
        }
        (_, PixelLayout::I422) => (width.div_ceil(2), height),
        (_, PixelLayout::I420) => (width.div_ceil(2), height.div_ceil(2)),
    }
}

enum Conversion {
    Gbr,
    YCgCo,
    Matrix(YuvStandardMatrix),
}

fn to_rgba(picture: &Picture, rgba: &mut [u8]) -> Result<(), AvifError> {
    let full = picture.color_range() == YUVRange::Full;
    let y = plane_8bit(picture, PlanarImageComponent::Y);
    if picture.pixel_layout() == PixelLayout::I400 {
        for (pixel, &value) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(&y) {
            let gray = luma(value, full);
            pixel[..3].fill(gray);
        }
        return Ok(());
    }

    let u = plane_8bit(picture, PlanarImageComponent::U);
    let v = plane_8bit(picture, PlanarImageComponent::V);
    let (width, height) = (picture.width(), picture.height());
    let chroma_width = plane_size(picture, PlanarImageComponent::U).0 as u32;
    let planar = YuvPlanarImage {
        y_plane: &y,
        y_stride: width,
        u_plane: &u,
        u_stride: chroma_width,
        v_plane: &v,
        v_stride: chroma_width,
        width,
        height,
    };
    let range = if full {
        YuvRange::Full
    } else {
        YuvRange::Limited
    };
    let stride = width * 4;
    let conversion = match picture.matrix_coefficients() {
        MatrixCoefficients::Identity => Conversion::Gbr,
        MatrixCoefficients::YCgCo => Conversion::YCgCo,
        MatrixCoefficients::BT709 => Conversion::Matrix(YuvStandardMatrix::Bt709),
        MatrixCoefficients::BT470M => Conversion::Matrix(YuvStandardMatrix::Fcc),
        MatrixCoefficients::BT470BG | MatrixCoefficients::Unspecified => {
            Conversion::Matrix(YuvStandardMatrix::Bt601)
        }
        MatrixCoefficients::ST240M => Conversion::Matrix(YuvStandardMatrix::Smpte240),
        MatrixCoefficients::BT2020NonConstantLuminance
        | MatrixCoefficients::BT2020ConstantLuminance => {
            Conversion::Matrix(YuvStandardMatrix::Bt2020)
        }
        _ => return Err(AvifError::Unsupported("these matrix coefficients")),
    };

    match (conversion, picture.pixel_layout()) {
        (Conversion::Gbr, PixelLayout::I444) => yuv::gbr_to_rgba(&planar, rgba, stride, range)?,
        (Conversion::Gbr, _) => {
            return Err(AvifError::Unsupported(
                "an identity matrix with chroma subsampling",
            ));
        }
        (Conversion::YCgCo, PixelLayout::I444) => {
            yuv::ycgco444_to_rgba(&planar, rgba, stride, range)?
        }
        (Conversion::YCgCo, PixelLayout::I422) => {
            yuv::ycgco422_to_rgba(&planar, rgba, stride, range)?
        }
        (Conversion::YCgCo, _) => yuv::ycgco420_to_rgba(&planar, rgba, stride, range)?,
        (Conversion::Matrix(matrix), PixelLayout::I444) => {
            yuv::yuv444_to_rgba(&planar, rgba, stride, range, matrix)?
        }
        (Conversion::Matrix(matrix), PixelLayout::I422) => {
            yuv::yuv422_to_rgba(&planar, rgba, stride, range, matrix)?
        }
        (Conversion::Matrix(matrix), _) => {
            yuv::yuv420_to_rgba(&planar, rgba, stride, range, matrix)?
        }
    }

    Ok(())
}

fn luma(value: u8, full: bool) -> u8 {
    match full {
        true => value,
        false => ((i32::from(value) - 16) * 255 / 219).clamp(0, 255) as u8,
    }
}

fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_range_luma_is_stretched() {
        assert_eq!(luma(16, false), 0);
        assert_eq!(luma(235, false), 255);
        assert_eq!(luma(0, false), 0);
        assert_eq!(luma(100, true), 100);
    }

    #[test]
    fn premultiplied_colors_are_restored() {
        let mut rgba = vec![64, 32, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0];

        unpremultiply(&mut rgba);

        assert_eq!(rgba, vec![127, 63, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0]);
    }

    #[test]
    fn something_that_is_not_avif_is_an_error() {
        assert!(matches!(
            decode(b"not an image"),
            Err(AvifError::Container(_))
        ));
    }
}
