use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use super::{MAX_BINS, Waveform};

pub struct Cache {
    connection: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(1))?;
        connection.execute_batch("PRAGMA max_page_count=32768; CREATE TABLE IF NOT EXISTS waveforms(path BLOB PRIMARY KEY, size INTEGER NOT NULL, modified TEXT NOT NULL, algorithm INTEGER NOT NULL, rate INTEGER NOT NULL, frames INTEGER NOT NULL, span INTEGER NOT NULL, peaks BLOB NOT NULL, used INTEGER NOT NULL) STRICT; CREATE INDEX IF NOT EXISTS waveform_used ON waveforms(used);")?;
        Ok(Self { connection })
    }

    pub fn get(
        &mut self,
        path: &Path,
    ) -> Result<Option<Waveform>, Box<dyn std::error::Error + Send + Sync>> {
        let metadata = path.metadata()?;
        let modified = format!("{:?}", metadata.modified()?);
        let key = crate::path_codec::encode(path);
        let record = self.connection.query_row(
            "SELECT rate, frames, span, peaks FROM waveforms WHERE path=?1 AND size=?2 AND modified=?3 AND algorithm=1",
            params![key, i64::try_from(metadata.len())?, modified],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?)),
        ).optional()?;
        let Some((rate, frames, frames_per_bin, bytes)) = record else {
            return Ok(None);
        };
        let (Ok(frames), Ok(frames_per_bin)) =
            (u64::try_from(frames), u64::try_from(frames_per_bin))
        else {
            return Ok(None);
        };
        if rate == 0
            || frames == 0
            || frames_per_bin == 0
            || !bytes.len().is_multiple_of(8)
            || bytes.is_empty()
            || bytes.len() > (MAX_BINS + 1) * 8
        {
            return Ok(None);
        }
        if frames.div_ceil(frames_per_bin) != (bytes.len() / 8) as u64 {
            return Ok(None);
        }
        let mut peaks = Vec::with_capacity(bytes.len() / 8);
        for chunk in bytes.as_chunks::<8>().0 {
            let low = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let high = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            if !low.is_finite() || !high.is_finite() || low > high {
                return Ok(None);
            }
            peaks.push([low, high]);
        }
        self.connection.execute(
            "UPDATE waveforms SET used=?2 WHERE path=?1",
            params![key, clock()],
        )?;
        Ok(Some(Waveform {
            rate,
            frames,
            frames_per_bin,
            peaks,
            source_size: metadata.len(),
            source_modified: metadata.modified()?,
        }))
    }

    pub fn save(
        &mut self,
        path: &Path,
        waveform: &Waveform,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let metadata = path.metadata()?;
        if metadata.len() != waveform.source_size
            || metadata.modified()? != waveform.source_modified
        {
            return Err(
                std::io::Error::other("Source changed before waveform cache publication").into(),
            );
        }
        let modified = format!("{:?}", metadata.modified()?);
        let mut bytes = Vec::with_capacity(waveform.peaks.len() * 8);
        for peak in &waveform.peaks {
            bytes.extend(peak[0].to_le_bytes());
            bytes.extend(peak[1].to_le_bytes());
        }
        let transaction = self.connection.transaction()?;
        // Bound both entry count and peak payload before adding the next entry.
        transaction.execute("DELETE FROM waveforms WHERE path IN (SELECT path FROM waveforms ORDER BY used DESC LIMIT -1 OFFSET 4095)", [])?;
        let payload: i64 = transaction.query_row(
            "SELECT coalesce(sum(length(peaks)),0) FROM waveforms",
            [],
            |row| row.get(0),
        )?;
        if payload > 64 * 1024 * 1024 - 16_392 {
            transaction.execute("DELETE FROM waveforms WHERE path IN (SELECT path FROM waveforms ORDER BY used LIMIT 16)", [])?;
        }
        transaction.execute("INSERT INTO waveforms VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8) ON CONFLICT(path) DO UPDATE SET size=excluded.size, modified=excluded.modified, algorithm=1, rate=excluded.rate, frames=excluded.frames, span=excluded.span, peaks=excluded.peaks, used=excluded.used", params![crate::path_codec::encode(path), i64::try_from(metadata.len())?, modified, waveform.rate, i64::try_from(waveform.frames)?, i64::try_from(waveform.frames_per_bin)?, bytes, clock()])?;
        transaction.commit()?;
        Ok(())
    }
}

fn clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
