use anyhow::Result;
use num_complex::Complex32;
use std::fs::File;
use std::io::{BufWriter, Write};


#[allow(dead_code)]
pub struct IqWriter {
    writer: BufWriter<File>,
    writer_demod: BufWriter<File>,
}

#[allow(dead_code)]
impl IqWriter {

    pub fn create(path: &str, path2: &str) -> Result<Self> {
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
            writer_demod: BufWriter::new(File::create(path2)?),
        })
    }

    pub fn write_samples(
        &mut self,
        samples: &[Complex32],
    ) -> Result<()> {
        for s in samples {
            self.writer.write_all(&s.re.to_le_bytes())?;
            self.writer.write_all(&s.im.to_le_bytes())?;
        }

        Ok(())
    }

    pub fn write_demod(&mut self, time_offset: f32, samples: Complex32, demod_sample: f32) -> Result<()> {
        self.writer_demod.write_all(&time_offset.to_le_bytes())?;
        self.writer_demod.write_all(&samples.re.to_le_bytes())?;
        self.writer_demod.write_all(&samples.im.to_le_bytes())?;
        self.writer_demod.write_all(&demod_sample.to_le_bytes())?;

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer_demod.flush()?;
        Ok(())
    }

}
