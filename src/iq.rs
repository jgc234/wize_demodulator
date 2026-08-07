use num_complex::Complex32;

pub fn iq_to_complex(raw: &[u8], out: &mut Vec<Complex32>) {
    out.clear();

    for iq in raw.chunks_exact(2) {
        let i = (iq[0] as f32 - 127.5) / 127.5;
        let q = (iq[1] as f32 - 127.5) / 127.5;

        out.push(Complex32::new(i, q));
    }
}