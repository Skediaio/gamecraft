//! The heightmap of a world of a project on disk: `worlds/<world>/terrain/heightmap/`
//! `tile_<tx>_<tz>.bin`, one file per tile of 128 cells (129 x 129 samples a side, shared
//! borders), beside the edits and the derived field's cache (`terrain/cache`, which is thrown
//! away at will; the heightmap is a source and is committed). The document's `world.json`
//! names the heightmap by its sample count and a hash of its content, so a tile set that does
//! not match is an error, never a guess.
//!
//! A file is the 10 bytes `GCHEIGHT1\n`, the tile's x and z (i32 each, little-endian), then
//! `TILE_SAMPLES` heights (f32, little-endian), row-major from the north-west corner.

use gamecraft_world::Heightmap;
use gamecraft_world::heightmap::TILE_SAMPLES;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The first bytes of a tile file.
pub const TILE_HEADER: &[u8; 10] = b"GCHEIGHT1\n";
/// Bytes before the samples: the header and the tile's place.
pub const TILE_HEADER_BYTES: usize = TILE_HEADER.len() + 8;
/// One tile file whole.
pub const TILE_BYTES: usize = TILE_HEADER_BYTES + TILE_SAMPLES * 4;

/// The name of the heightmap's folder in a world's `terrain/` folder.
pub const HEIGHTMAP_DIR: &str = "heightmap";

/// The file name of tile `(tx, tz)`.
pub fn tile_file_name(tx: i32, tz: i32) -> String {
    format!("tile_{tx}_{tz}.bin")
}

/// The bytes of tile `(tx, tz)` holding `samples` ([`TILE_SAMPLES`] heights).
pub fn encode_tile(tx: i32, tz: i32, samples: &[f32]) -> Result<Vec<u8>, String> {
    if samples.len() != TILE_SAMPLES {
        return Err(format!(
            "a tile holds {TILE_SAMPLES} samples, not {}",
            samples.len()
        ));
    }
    let mut out = Vec::with_capacity(TILE_BYTES);
    out.extend_from_slice(TILE_HEADER);
    out.extend_from_slice(&tx.to_le_bytes());
    out.extend_from_slice(&tz.to_le_bytes());
    for h in samples {
        out.extend_from_slice(&h.to_le_bytes());
    }
    Ok(out)
}

/// A tile read from the bytes of its file.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    /// `(tx, tz)`: the tile's place on the grid.
    pub place: (i32, i32),
    /// Its samples, row-major from the tile's north-west corner.
    pub samples: Arc<[f32]>,
}

/// The tile in the bytes of its file. An error, never a panic, for another header, a short
/// file or a height that is not a finite number.
pub fn decode_tile(bytes: &[u8]) -> Result<Tile, String> {
    if bytes.get(..TILE_HEADER.len()) != Some(&TILE_HEADER[..]) {
        return Err("the file does not start with GCHEIGHT1 (not a heightmap tile)".into());
    }
    let word = |at: usize| -> Result<i32, String> {
        let b = bytes
            .get(at..at + 4)
            .ok_or_else(|| "the tile file is cut short".to_string())?;
        Ok(i32::from_le_bytes([
            *b.first().ok_or("cut short")?,
            *b.get(1).ok_or("cut short")?,
            *b.get(2).ok_or("cut short")?,
            *b.get(3).ok_or("cut short")?,
        ]))
    };
    let tx = word(TILE_HEADER.len())?;
    let tz = word(TILE_HEADER.len() + 4)?;
    if bytes.len() != TILE_BYTES {
        return Err(format!(
            "the tile file holds {} bytes, {TILE_BYTES} expected",
            bytes.len()
        ));
    }
    let mut samples = Vec::with_capacity(TILE_SAMPLES);
    let raster = bytes
        .get(TILE_HEADER_BYTES..)
        .ok_or_else(|| "the tile file is cut short".to_string())?;
    for h in raster
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
    {
        if !h.is_finite() {
            return Err("the tile holds a height that is not a finite number".into());
        }
        samples.push(h);
    }
    Ok(Tile {
        place: (tx, tz),
        samples: Arc::from(samples),
    })
}

/// The tiles of the heightmap, by place, each as the bytes of its file.
pub fn encode_tiles(map: &Heightmap) -> BTreeMap<(i32, i32), Vec<u8>> {
    map.tiles()
        .iter()
        .map(|(&(tx, tz), samples)| ((tx, tz), encode_tile(tx, tz, samples).unwrap_or_default()))
        .collect()
}

/// The format of an elevation file `terrain.import` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElevationFormat {
    /// A PGM (P5): 8- or 16-bit samples, big-endian; raw numbers, with `scale_m` and `offset_m`.
    Pgm,
    /// Rows of text, one row of the heightmap per line, metres.
    Text,
    /// 16-bit unsigned integers, little-endian; raw numbers, with `scale_m` and `offset_m`.
    U16,
    /// 32-bit floats, little-endian, metres.
    F32,
}

impl ElevationFormat {
    /// The format a file's extension names (lowercase); `None` for any other extension.
    pub fn by_extension(path: &str) -> Option<ElevationFormat> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "pgm" => Some(ElevationFormat::Pgm),
            "txt" => Some(ElevationFormat::Text),
            "u16" => Some(ElevationFormat::U16),
            "f32" => Some(ElevationFormat::F32),
            _ => None,
        }
    }

    /// The format its `format` parameter names.
    pub fn parse(name: &str) -> Option<ElevationFormat> {
        match name {
            "pgm" => Some(ElevationFormat::Pgm),
            "text" => Some(ElevationFormat::Text),
            "u16" => Some(ElevationFormat::U16),
            "f32" => Some(ElevationFormat::F32),
            _ => None,
        }
    }

    /// Its name in commands and answers.
    pub fn name(self) -> &'static str {
        match self {
            ElevationFormat::Pgm => "pgm",
            ElevationFormat::Text => "text",
            ElevationFormat::U16 => "u16",
            ElevationFormat::F32 => "f32",
        }
    }

    /// Whether its numbers are raw and take `scale_m` and `offset_m` (height = raw * scale +
    /// offset).
    pub fn takes_scale(self) -> bool {
        matches!(self, ElevationFormat::Pgm | ElevationFormat::U16)
    }

    /// Whether `cols` and `rows` must be given (the file does not say).
    pub fn needs_shape(self) -> bool {
        matches!(self, ElevationFormat::U16 | ElevationFormat::F32)
    }
}

/// The largest elevation file read, bytes (2049 x 2049 samples as text take under 40 MB).
pub const MAX_ELEVATION_BYTES: u64 = 128 * 1024 * 1024;

/// `cols` or `rows` as a parameter of a raw file gives it.
fn shape_param(name: &str, value: Option<u32>) -> Result<u32, String> {
    let Some(value) = value else {
        return Err(format!(
            "`{name}` is required for a file of raw samples, which does not say how many there are a side; give `{name}` and the other one"
        ));
    };
    if !(gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN
        ..=gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX)
        .contains(&value)
    {
        return Err(format!(
            "`{name}` is {value}; it must be from {} to {}",
            gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN,
            gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX
        ));
    }
    Ok(value)
}

/// The heightmap of an elevation file in the given `format`: `cols` and `rows` for a file of
/// raw samples (`None` is an error for those, ignored for the others), and `scale_m` and
/// `offset_m` for the formats whose numbers are raw. An error, never a panic, for a file that
/// is not what it claims to be.
pub fn read_elevation(
    format: ElevationFormat,
    bytes: &[u8],
    cols: Option<u32>,
    rows: Option<u32>,
    scale_m: f64,
    offset_m: f64,
) -> Result<Heightmap, String> {
    if !(scale_m.is_finite() && offset_m.is_finite()) {
        return Err("`scale_m` and `offset_m` must be finite numbers".into());
    }
    let shape = || -> Result<(u32, u32), String> {
        Ok((shape_param("cols", cols)?, shape_param("rows", rows)?))
    };
    match format {
        ElevationFormat::Pgm => read_pgm(bytes, scale_m, offset_m),
        ElevationFormat::Text => read_text(bytes),
        ElevationFormat::U16 => {
            let (cols, rows) = shape()?;
            read_u16(bytes, cols, rows, scale_m, offset_m)
        }
        ElevationFormat::F32 => {
            let (cols, rows) = shape()?;
            read_f32(bytes, cols, rows)
        }
    }
}

/// A PGM (P5) being read: `P5`, whitespace, the width, the height and the largest sample
/// (each may be followed by a `#` comment), one whitespace character, then the samples.
struct Pgm<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Pgm<'_> {
    /// The next token of the header from `at`: whitespace and comments skipped.
    fn token(&mut self) -> Result<&[u8], String> {
        loop {
            match self.bytes.get(self.at).copied() {
                Some(b'#') => {
                    while let Some(&b) = self.bytes.get(self.at)
                        && b != b'\n'
                    {
                        self.at += 1;
                    }
                }
                Some(b) if b.is_ascii_whitespace() => self.at += 1,
                Some(_) => break,
                None => return Err("the PGM header is cut short".to_string()),
            }
        }
        let start = self.at;
        while let Some(&b) = self.bytes.get(self.at)
            && !b.is_ascii_whitespace()
            && b != b'#'
        {
            self.at += 1;
        }
        self.bytes
            .get(start..self.at)
            .ok_or_else(|| "the PGM header is cut short".to_string())
    }

    /// The next token as a whole number.
    fn number(&mut self, what: &str) -> Result<u64, String> {
        let text = std::str::from_utf8(self.token()?)
            .map_err(|_| format!("the PGM's {what} is not a number"))?;
        text.parse::<u64>()
            .map_err(|_| format!("the PGM's {what} is {text:?}, not a whole number"))
    }
}

/// The side the PGM's `what` says, within the heightmap's limits.
fn pgm_side(value: u64, what: &str) -> Result<u32, String> {
    u32::try_from(value)
        .ok()
        .filter(|n| {
            (gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN
                ..=gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX)
                .contains(n)
        })
        .ok_or_else(|| {
            format!(
                "the PGM's {what} is {value} samples; it must be from {} to {}",
                gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN,
                gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX
            )
        })
}

fn read_pgm(bytes: &[u8], scale_m: f64, offset_m: f64) -> Result<Heightmap, String> {
    if bytes.first() != Some(&b'P') || bytes.get(1) != Some(&b'5') {
        return Err("the file is not a PGM: it does not start with the two letters P5".into());
    }
    let mut pgm = Pgm { bytes, at: 2 };
    let width = pgm.number("width")?;
    let height = pgm.number("height")?;
    let maxval = pgm.number("largest sample (maxval)")?;
    if !(1..=65535).contains(&maxval) {
        return Err(format!(
            "the PGM's largest sample (maxval) is {maxval}; it must be from 1 to 65535"
        ));
    }
    // Exactly one whitespace character ends the header.
    match bytes.get(pgm.at) {
        Some(b) if b.is_ascii_whitespace() => pgm.at += 1,
        _ => return Err("the PGM header is cut short".to_string()),
    }
    let (width, height) = (pgm_side(width, "width")?, pgm_side(height, "height")?);
    // Below 256 a sample is one byte, else two, big-endian.
    let per = if maxval > 255 { 2 } else { 1 };
    let count = u64::from(width) * u64::from(height);
    let raster = bytes
        .get(pgm.at..)
        .ok_or_else(|| "the PGM is cut short".to_string())?;
    let held = (raster.len() / per) as u64;
    if held != count {
        return Err(format!(
            "the PGM holds {held} samples; {width} x {height} = {count} expected (a sample is \
             {per} byte{}: the largest sample is {maxval})",
            if per == 2 { "s" } else { "" }
        ));
    }
    Heightmap::from_samples(width, height, |c, r| {
        let i = (r as usize * width as usize + c as usize) * per;
        let raw = match raster.get(i..i + per) {
            Some([a]) => u64::from(*a),
            Some([a, b]) => (u64::from(*a) << 8) | u64::from(*b),
            _ => 0,
        };
        raw as f64 * scale_m + offset_m
    })
}

/// Rows of text: one row of the heightmap per line, samples separated by spaces or tabs;
/// every row as long as the first.
fn read_text(bytes: &[u8]) -> Result<Heightmap, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "the file is not text (UTF-8); give its format with `format`".to_string())?;
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .filter(|row| !row.is_empty())
        .collect();
    let Some(first) = rows.first() else {
        return Err("the file holds no samples".into());
    };
    for (r, row) in rows.iter().enumerate() {
        if row.len() != first.len() {
            return Err(format!(
                "row {} holds {} numbers, row 1 holds {}; every row of a heightmap is as long \
                 as the first",
                r + 1,
                row.len(),
                first.len()
            ));
        }
    }
    let side = |n: usize, what: &str| {
        u32::try_from(n)
            .ok()
            .filter(|n| {
                (gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN
                    ..=gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX)
                    .contains(n)
            })
            .ok_or_else(|| {
                format!(
                    "the file has {n} {what}; it must be from {} to {}",
                    gamecraft_world::heightmap::HEIGHTMAP_SIDE_MIN,
                    gamecraft_world::heightmap::HEIGHTMAP_SIDE_MAX
                )
            })
    };
    let (cols, count) = (
        side(first.len(), "samples in a row")?,
        side(rows.len(), "rows")?,
    );
    // Every sample a finite number, said in its place otherwise.
    for (r, row) in rows.iter().enumerate() {
        for (c, word) in row.iter().enumerate() {
            if !word.parse::<f64>().is_ok_and(f64::is_finite) {
                return Err(format!(
                    "the sample at column {}, row {} is {word:?}; every sample is a finite \
                     number of metres",
                    c + 1,
                    r + 1
                ));
            }
        }
    }
    Heightmap::from_samples(cols, count, |c, r| {
        rows.get(r as usize)
            .and_then(|row| row.get(c as usize))
            .and_then(|w| w.parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    })
}

fn read_u16(
    bytes: &[u8],
    cols: u32,
    rows: u32,
    scale_m: f64,
    offset_m: f64,
) -> Result<Heightmap, String> {
    let count = u64::from(cols) * u64::from(rows);
    let held = u64::try_from(bytes.len() / 2).unwrap_or(u64::MAX);
    if held != count {
        return Err(format!(
            "the file holds {held} samples of 16 bits; `cols` x `rows` = {cols} x {rows} = \
             {count} expected"
        ));
    }
    Heightmap::from_samples(cols, rows, |c, r| {
        let i = (r as usize * cols as usize + c as usize) * 2;
        match bytes.get(i..i + 2) {
            Some([a, b]) => f64::from(u16::from_le_bytes([*a, *b])) * scale_m + offset_m,
            _ => f64::NAN,
        }
    })
}

fn read_f32(bytes: &[u8], cols: u32, rows: u32) -> Result<Heightmap, String> {
    let count = u64::from(cols) * u64::from(rows);
    let held = u64::try_from(bytes.len() / 4).unwrap_or(u64::MAX);
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != count.saturating_mul(4) {
        return Err(format!(
            "the file holds {held} samples of 32 bits; `cols` x `rows` = {cols} x {rows} = \
             {count} expected"
        ));
    }
    Heightmap::from_samples(cols, rows, |c, r| {
        let i = (r as usize * cols as usize + c as usize) * 4;
        match bytes.get(i..i + 4) {
            Some([a, b, c, d]) => f64::from(f32::from_le_bytes([*a, *b, *c, *d])),
            _ => f64::NAN,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_round_trips_and_says_what_is_wrong() {
        let map = Heightmap::from_samples(200, 130, |c, r| c as f64 + r as f64 / 10.0).unwrap();
        let files = encode_tiles(&map);
        assert_eq!(files.len(), 2 * 2);
        for (&(tx, tz), bytes) in &files {
            assert_eq!(bytes.len(), TILE_BYTES);
            let tile = decode_tile(bytes).unwrap();
            assert_eq!(tile.place, (tx, tz));
            for r in 0..=128usize {
                for c in 0..=128usize {
                    let (sc, sr) = (
                        (tx as usize * 128 + c) as u32,
                        (tz as usize * 128 + r) as u32,
                    );
                    if sc < 200 && sr < 130 {
                        let want = sc as f32 + sr as f32 / 10.0;
                        assert_eq!(tile.samples[r * 129 + c], want);
                    }
                }
            }
        }
        assert_eq!(tile_file_name(1, -2), "tile_1_-2.bin");
        assert!(encode_tile(0, 0, &[0.0; 10]).is_err());
        let good = &files[&(0, 0)];
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(decode_tile(&bad).unwrap_err().contains("GCHEIGHT1"));
        assert!(decode_tile(&good[..good.len() - 1]).is_err());
        assert!(decode_tile(&good[..8]).is_err());
        let mut nan = good.clone();
        nan[TILE_BYTES - 4..TILE_BYTES].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode_tile(&nan).unwrap_err().contains("finite"));
    }

    /// `cols`, `rows` as parameters, or none.
    fn elevation(
        format: ElevationFormat,
        bytes: &[u8],
        cols: u32,
        rows: u32,
        scale_m: f64,
        offset_m: f64,
    ) -> Result<Heightmap, String> {
        let shape = format.needs_shape().then_some((cols, rows));
        read_elevation(
            format,
            bytes,
            shape.map(|s| s.0),
            shape.map(|s| s.1),
            scale_m,
            offset_m,
        )
    }

    #[test]
    fn every_format_reads_the_same_little_heightmap() {
        // 2 x 2: 1, 2 / 3, 4 metres.
        let text = b"1 2\n3 4\n";
        let want = elevation(ElevationFormat::Text, text, 0, 0, 1.0, 0.0).unwrap();
        assert_eq!((want.cols(), want.rows()), (2, 2));
        assert_eq!(want.height_range(), (1.0, 4.0));
        // Raw 16-bit, scaled: (h - offset) / scale = raw.
        let mut u16s = Vec::new();
        for h in [1.0f64, 2.0, 3.0, 4.0] {
            u16s.extend_from_slice(&((h * 10.0) as u16).to_le_bytes());
        }
        let got = elevation(ElevationFormat::U16, &u16s, 2, 2, 0.1, 0.0).unwrap();
        assert_eq!(got.content_hash(), want.content_hash());
        let mut f32s = Vec::new();
        for h in [1.0f32, 2.0, 3.0, 4.0] {
            f32s.extend_from_slice(&h.to_le_bytes());
        }
        let got = elevation(ElevationFormat::F32, &f32s, 2, 2, 1.0, 0.0).unwrap();
        assert_eq!(got.content_hash(), want.content_hash());
        // A PGM of 8 bits and one of 16 bits.
        let pgm8 = b"P5\n#c\n 2 2\n255\n\x0a\x14\x1e\x28";
        let got = elevation(ElevationFormat::Pgm, pgm8, 0, 0, 0.1, 0.0).unwrap();
        assert_eq!(got.content_hash(), want.content_hash());
        let mut pgm16 = b"P5 2 2 65535\n".to_vec();
        for h in [1.0f64, 2.0, 3.0, 4.0] {
            pgm16.extend_from_slice(&((h * 10.0) as u16).to_be_bytes());
        }
        let got = elevation(ElevationFormat::Pgm, &pgm16, 0, 0, 0.1, 0.0).unwrap();
        assert_eq!(got.content_hash(), want.content_hash());
        // An offset.
        let got = elevation(ElevationFormat::Text, b"11 12\n13 14\n", 0, 0, 1.0, 0.0)
            .unwrap()
            .height_range();
        assert_eq!(got, (11.0, 14.0));
    }

    #[test]
    fn a_file_that_is_not_what_it_claims_is_an_error() {
        let say = |r: Result<Heightmap, String>, needle: &str| {
            let e = r.unwrap_err();
            assert!(e.contains(needle), "{needle} not in: {e}");
        };
        say(
            elevation(ElevationFormat::Pgm, b"P2 2 2 255", 0, 0, 1.0, 0.0),
            "P5",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5", 0, 0, 1.0, 0.0),
            "cut short",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 x 255\n....", 0, 0, 1.0, 0.0),
            "whole number",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 2 0\n....", 0, 0, 1.0, 0.0),
            "maxval",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 1 2 255\n..", 0, 0, 1.0, 0.0),
            "width",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 5000 255\n....", 0, 0, 1.0, 0.0),
            "height",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 2 255\n...", 0, 0, 1.0, 0.0),
            "3 samples",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 2 255\n.....", 0, 0, 1.0, 0.0),
            "5 samples",
        );
        say(
            elevation(ElevationFormat::Pgm, b"P5 2 2 65535\n....", 0, 0, 1.0, 0.0),
            "2 samples",
        );
        say(
            elevation(ElevationFormat::Text, b"", 0, 0, 1.0, 0.0),
            "no samples",
        );
        say(
            elevation(ElevationFormat::Text, b"1 2\n3\n", 0, 0, 1.0, 0.0),
            "row 2",
        );
        say(
            elevation(ElevationFormat::Text, b"1 2\nx 4\n", 0, 0, 1.0, 0.0),
            "column 1, row 2",
        );
        say(
            elevation(ElevationFormat::Text, b"1 nan", 0, 0, 1.0, 0.0),
            "2 to",
        );
        say(
            elevation(ElevationFormat::Text, b"1 1e300\n2 3\n", 0, 0, 1.0, 0.0),
            "1 000 000",
        );
        say(
            elevation(ElevationFormat::U16, &[0; 8], 0, 0, 1.0, 0.0),
            "`cols`",
        );
        say(
            elevation(ElevationFormat::U16, &[0; 8], 1, 2, 1.0, 0.0),
            "`cols` is 1",
        );
        say(
            elevation(ElevationFormat::U16, &[0; 10], 2, 2, 1.0, 0.0),
            "5 samples",
        );
        say(
            elevation(ElevationFormat::F32, &[0; 15], 2, 2, 1.0, 0.0),
            "3 samples",
        );
        say(
            elevation(
                ElevationFormat::F32,
                &f32::NAN.to_le_bytes(),
                1,
                2,
                1.0,
                0.0,
            ),
            "`cols`",
        );
        say(
            read_elevation(
                ElevationFormat::Text,
                b"1 2\n3 4\n",
                None,
                None,
                f64::NAN,
                0.0,
            ),
            "finite",
        );
        assert_eq!(
            ElevationFormat::by_extension("dem.PGM"),
            Some(ElevationFormat::Pgm)
        );
        assert_eq!(ElevationFormat::by_extension("dem.tif"), None);
        assert_eq!(ElevationFormat::by_extension("dem"), None);
        assert_eq!(ElevationFormat::parse("f32"), Some(ElevationFormat::F32));
        assert_eq!(ElevationFormat::parse("tif"), None);
        assert!(ElevationFormat::U16.takes_scale() && !ElevationFormat::Text.takes_scale());
    }
}
