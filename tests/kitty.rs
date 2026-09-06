use std::{ffi::OsString, io, num::NonZeroU32, sync::Arc, time::Duration};

use pcx_cli::core::point::{
    Endianness, PointDimensions, PointField, PointFieldSemantic, PointFrameMetadata, PointLayout,
    PointSchema, PointView, PrimitiveType, Timestamp,
};
use pcx_cli::{
    core::{Cancellation, LossPolicy, PointRepresentation},
    ops::{
        ColorPolicy, DepthPolicy, IntensityRange, InvalidProjectionCoordinatePolicy,
        OrthographicView, Projection, Raster, RasterDimensions, Rgb8,
    },
    terminal::{
        Backend, BackendChoice, CapabilityQuery, DetectionContext, QueryResult,
        kitty::{
            KITTY_CHUNK_BYTES, KITTY_ENCODER_BUFFER_BYTES, KittyEncoder, KittyError, KittyLimits,
            KittyWriteOutcome,
        },
        select_backend,
    },
};

struct Context {
    stdout_tty: bool,
    term: Option<OsString>,
}

impl DetectionContext for Context {
    fn stdout_is_terminal(&self) -> bool {
        self.stdout_tty
    }

    fn stdin_is_terminal(&self) -> bool {
        true
    }

    fn environment(&self, name: &str) -> Option<OsString> {
        (name == "TERM").then(|| self.term.clone()).flatten()
    }
}

struct NoQuery;

impl CapabilityQuery for NoQuery {
    fn query(&self, _timeout: Duration) -> QueryResult {
        panic!("explicit backend selection must not query")
    }
}

fn selection(choice: BackendChoice, stdout_tty: bool) -> pcx_cli::terminal::Selection {
    select_backend(
        choice,
        &Context {
            stdout_tty,
            term: None,
        },
        Arc::new(NoQuery),
    )
    .unwrap()
}

fn raster(width: usize) -> Raster {
    let schema = Arc::new(
        PointSchema::new(vec![
            PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
            PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
            PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
            PointField::new(
                "intensity",
                PrimitiveType::F32,
                1,
                Some(PointFieldSemantic::Intensity),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let points = [(-1.0_f64, 0.0_f64, 0.0_f64, 0.0_f32), (1.0, 0.0, 0.0, 1.0)];
    let mut source = Vec::new();
    for (x, y, z, intensity) in points {
        source.extend_from_slice(&x.to_le_bytes());
        source.extend_from_slice(&y.to_le_bytes());
        source.extend_from_slice(&z.to_le_bytes());
        source.extend_from_slice(&intensity.to_le_bytes());
    }
    let dimensions = PointDimensions::new(points.len(), 1).unwrap();
    let view = PointView::new(
        Arc::from(source),
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(1, 2).unwrap(),
            "map",
            false,
        )),
        PointLayout::new(
            Arc::clone(&schema),
            dimensions,
            vec![0, 8, 16, 24],
            28,
            56,
            0,
            Endianness::Little,
        )
        .unwrap(),
    )
    .unwrap();
    let projection = Projection::new(
        RasterDimensions::new(width, 1).unwrap(),
        OrthographicView::xy(),
        DepthPolicy::Nearest,
        InvalidProjectionCoordinatePolicy::Drop,
        ColorPolicy::Intensity {
            range: IntensityRange::new(0.0, 1.0).unwrap(),
            invalid: Rgb8([255, 0, 255]),
        },
    );
    projection
        .plan(
            schema,
            dimensions,
            PointRepresentation::View,
            &LossPolicy::lossless(),
        )
        .unwrap()
        .execute_view(&view)
        .unwrap()
}

fn encoder(limits: KittyLimits) -> KittyEncoder {
    KittyEncoder::new(NonZeroU32::new(28).unwrap(), limits)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2 + bytes.len() / 32);
    for (index, byte) in bytes.iter().copied().enumerate() {
        if index > 0 && index % 32 == 0 {
            output.push('\n');
        }
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    output.push('\n');
    output
}

#[test]
fn exact_escape_bytes_match_the_reviewed_golden() {
    let raster = raster(3);
    let encoder = encoder(KittyLimits::default());
    let plan = encoder.plan(&raster).unwrap();
    let mut output = Vec::new();

    let outcome = encoder
        .write(
            selection(BackendChoice::Kitty, true),
            &raster,
            &Cancellation::default(),
            &mut output,
        )
        .unwrap();

    assert_eq!(
        outcome,
        KittyWriteOutcome::Rendered {
            chunks: 1,
            payload_bytes: 16,
            output_bytes: output.len(),
        }
    );
    assert_eq!(plan.output_bytes(), output.len());
    assert_eq!(hex(&output), include_str!("golden/kitty_rgba_3x1.hex"));
}

#[test]
fn payload_chunks_and_encoder_memory_are_strictly_bounded() {
    let raster = raster(769);
    let encoder = encoder(KittyLimits::default());
    let plan = encoder.plan(&raster).unwrap();
    assert_eq!(plan.raw_bytes(), 769 * 4);
    assert_eq!(plan.payload_bytes(), 4104);
    assert_eq!(plan.chunks(), 2);
    assert_eq!(
        encoder.memory_bound(),
        pcx_cli::core::ByteBound::bounded(7298)
    );
    assert_eq!(KITTY_ENCODER_BUFFER_BYTES, 7298);

    let mut output = Vec::new();
    encoder
        .write(
            selection(BackendChoice::Kitty, true),
            &raster,
            &Cancellation::default(),
            &mut output,
        )
        .unwrap();
    let commands: Vec<&[u8]> = output
        .split(|byte| *byte == b'\\')
        .filter(|command| !command.is_empty())
        .collect();
    assert_eq!(commands.len(), 2);
    let payload_lengths: Vec<usize> = commands
        .iter()
        .map(|command| command.iter().position(|byte| *byte == b';').unwrap())
        .zip(commands.iter())
        .map(|(separator, command)| command.len() - separator - 2)
        .collect();
    assert_eq!(payload_lengths, [KITTY_CHUNK_BYTES, 8]);
    assert!(commands[0].windows(4).any(|bytes| bytes == b"m=1;"));
    assert!(commands[1].windows(4).any(|bytes| bytes == b"m=0,"));
}

#[test]
fn configured_dimension_and_payload_limits_fail_before_output() {
    let raster = raster(3);
    let cases = [
        encoder(KittyLimits::new(2, 1, u64::MAX)),
        encoder(KittyLimits::new(3, 1, 15)),
    ];
    for encoder in cases {
        let mut output = Vec::new();
        let error = encoder
            .write(
                selection(BackendChoice::Kitty, true),
                &raster,
                &Cancellation::default(),
                &mut output,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            KittyError::DimensionsExceeded { .. } | KittyError::PayloadLimitExceeded { .. }
        ));
        assert!(output.is_empty());
    }
}

#[test]
fn portable_fallback_selections_emit_no_escape_or_other_bytes() {
    let raster = raster(3);
    let encoder = encoder(KittyLimits::default());
    for (backend, stdout_tty) in [
        (BackendChoice::Sixel, true),
        (BackendChoice::Unicode, true),
        (BackendChoice::Plain, false),
    ] {
        let selected = selection(backend, stdout_tty);
        let mut output = Vec::new();
        let outcome = encoder
            .write(selected, &raster, &Cancellation::default(), &mut output)
            .unwrap();
        assert_eq!(outcome, KittyWriteOutcome::Fallback(selected.backend()));
        assert!(output.is_empty());
        assert!(!output.contains(&0x1b));
    }
}

struct UnsupportedQuery;

impl CapabilityQuery for UnsupportedQuery {
    fn query(&self, _timeout: Duration) -> QueryResult {
        QueryResult::Unsupported
    }
}

#[test]
fn environment_claim_alone_never_authorizes_kitty_escape_output() {
    let selected = select_backend(
        BackendChoice::Auto,
        &Context {
            stdout_tty: true,
            term: Some("xterm-kitty".into()),
        },
        Arc::new(UnsupportedQuery),
    )
    .unwrap();
    assert_eq!(selected.backend(), Backend::Unicode);

    let mut output = Vec::new();
    let outcome = encoder(KittyLimits::default())
        .write(selected, &raster(3), &Cancellation::default(), &mut output)
        .unwrap();
    assert_eq!(outcome, KittyWriteOutcome::Fallback(Backend::Unicode));
    assert!(output.is_empty());
}

struct CancelAfterWrite {
    output: Vec<u8>,
    cancellation: Cancellation,
    writes: usize,
}

impl io::Write for CancelAfterWrite {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(bytes);
        self.writes += 1;
        if self.writes == 1 {
            self.cancellation.cancel();
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn interruption_before_output_is_silent_and_midstream_is_image_scoped_cleanup() {
    let raster = raster(3);
    let encoder = encoder(KittyLimits::default());
    let selected = selection(BackendChoice::Kitty, true);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let mut untouched = Vec::new();
    assert!(matches!(
        encoder.write(selected, &raster, &cancelled, &mut untouched),
        Err(KittyError::Interrupted)
    ));
    assert!(untouched.is_empty());

    let cancellation = Cancellation::default();
    let mut writer = CancelAfterWrite {
        output: Vec::new(),
        cancellation: cancellation.clone(),
        writes: 0,
    };
    assert!(matches!(
        encoder.write(selected, &raster, &cancellation, &mut writer),
        Err(KittyError::Interrupted)
    ));
    assert!(writer.output.ends_with(b"\x1b_Ga=d,d=I,i=28,q=2\x1b\\"));
    assert!(!writer.output.windows(6).any(|bytes| bytes == b"a=d,d=A"));
}

#[test]
fn explicit_delete_is_also_gated_by_the_selected_backend() {
    let encoder = encoder(KittyLimits::default());
    let mut output = Vec::new();
    assert_eq!(
        encoder
            .delete(selection(BackendChoice::Plain, false), &mut output)
            .unwrap(),
        KittyWriteOutcome::Fallback(Backend::Plain)
    );
    assert!(output.is_empty());
    assert_eq!(
        encoder
            .delete(selection(BackendChoice::Kitty, true), &mut output)
            .unwrap(),
        KittyWriteOutcome::Deleted
    );
    assert_eq!(output, b"\x1b_Ga=d,d=I,i=28,q=2\x1b\\");
}
