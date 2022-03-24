use std::arch::x86_64::*;
use std::ops::{Add, Mul, Neg};
use std::cell::RefCell;
use rand::Rng;
use rand::distributions::Standard;

use crate::{arith::*, params::*, ntt::*, util::*, discrete_gaussian::*};

const SCRATCH_SPACE: usize = 8192;
thread_local!(static SCRATCH: RefCell<Vec<u64>> = RefCell::new(vec![0u64; SCRATCH_SPACE]));

pub trait PolyMatrix<'a> {
    fn is_ntt(&self) -> bool;
    fn get_rows(&self) -> usize;
    fn get_cols(&self) -> usize;
    fn get_params(&self) -> &Params;
    fn num_words(&self) -> usize;
    fn zero(params: &'a Params, rows: usize, cols: usize) -> Self;
    fn random(params: &'a Params, rows: usize, cols: usize) -> Self;
    fn as_slice(&self) -> &[u64];
    fn as_mut_slice(&mut self) -> &mut [u64];
    fn zero_out(&mut self) {
        for item in self.as_mut_slice() {
            *item = 0;
        }
    }
    fn get_poly(&self, row: usize, col: usize) -> &[u64] {
        let num_words = self.num_words();
        let start = (row * self.get_cols() + col) * num_words;
        &self.as_slice()[start..start + num_words]
    }
    fn get_poly_mut(&mut self, row: usize, col: usize) -> &mut [u64] {
        let num_words = self.num_words();
        let start = (row * self.get_cols() + col) * num_words;
        &mut self.as_mut_slice()[start..start + num_words]
    }
    fn copy_into(&mut self, p: &Self, target_row: usize, target_col: usize) {
        assert!(target_row < self.get_rows());
        assert!(target_col < self.get_cols());
        assert!(target_row + p.get_rows() < self.get_rows());
        assert!(target_col + p.get_cols() < self.get_cols());
        for r in 0..p.get_rows() {
            for c in 0..p.get_cols() {
                let pol_src = p.get_poly(r, c);
                let pol_dst = self.get_poly_mut(target_row + r, target_col + c);
                pol_dst.copy_from_slice(pol_src);
            }
        }
    }
    fn pad_top(&self, pad_rows: usize) -> Self;
}

pub struct PolyMatrixRaw<'a> {
    pub params: &'a Params,
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<u64>,
}

pub struct PolyMatrixNTT<'a> {
    pub params: &'a Params,
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<u64>,
}

impl<'a> PolyMatrix<'a> for PolyMatrixRaw<'a> {
    fn is_ntt(&self) -> bool {
        false
    }
    fn get_rows(&self) -> usize {
        self.rows
    }
    fn get_cols(&self) -> usize {
        self.cols
    }
    fn get_params(&self) -> &Params {
        &self.params
    }
    fn as_slice(&self) -> &[u64] {
        self.data.as_slice()
    }
    fn as_mut_slice(&mut self) -> &mut [u64] {
        self.data.as_mut_slice()
    }
    fn num_words(&self) -> usize {
        self.params.poly_len
    }
    fn zero(params: &'a Params, rows: usize, cols: usize) -> PolyMatrixRaw<'a> {
        let num_coeffs = rows * cols * params.poly_len;
        let data: Vec<u64> = vec![0; num_coeffs];
        PolyMatrixRaw {
            params,
            rows,
            cols,
            data,
        }
    }
    fn random(params: &'a Params, rows: usize, cols: usize) -> Self {
        let rng = rand::thread_rng();
        let mut iter = rng.sample_iter(&Standard);
        let mut out = PolyMatrixRaw::zero(params, rows, cols);
        for r in 0..rows {
            for c in 0..cols {
                for i in 0..params.poly_len {
                    let val: u64 = iter.next().unwrap();
                    out.get_poly_mut(r, c)[i] = val % params.modulus;
                }
            }
        }
        out
    }
    fn pad_top(&self, pad_rows: usize) -> Self {
        let mut padded = Self::zero(self.params, self.rows + pad_rows, self.cols);
        padded.copy_into(&self, pad_rows, 0);
        padded
    }
}

impl<'a> PolyMatrixRaw<'a> {
    pub fn identity(params: &'a Params, rows: usize, cols: usize) -> PolyMatrixRaw<'a> {
        let num_coeffs = rows * cols * params.poly_len;
        let mut data: Vec<u64> = vec![0; num_coeffs];
        for r in 0..rows {
            let c = r;
            let idx = r * cols * params.poly_len + c * params.poly_len;
            data[idx] = 1;
        }
        PolyMatrixRaw {
            params,
            rows,
            cols,
            data,
        }
    }

    pub fn noise(params: &'a Params, rows: usize, cols: usize, dg: &mut DiscreteGaussian) -> Self {
        let mut out = PolyMatrixRaw::zero(params, rows, cols);
        dg.sample_matrix(&mut out);
        out
    }

    pub fn ntt(&self) -> PolyMatrixNTT<'a> {
        to_ntt_alloc(&self)
    }
}

impl<'a> PolyMatrix<'a> for PolyMatrixNTT<'a> {
    fn is_ntt(&self) -> bool {
        true
    }
    fn get_rows(&self) -> usize {
        self.rows
    }
    fn get_cols(&self) -> usize {
        self.cols
    }
    fn get_params(&self) -> &Params {
        &self.params
    }
    fn as_slice(&self) -> &[u64] {
        self.data.as_slice()
    }
    fn as_mut_slice(&mut self) -> &mut [u64] {
        self.data.as_mut_slice()
    }
    fn num_words(&self) -> usize {
        self.params.poly_len * self.params.crt_count
    }
    fn zero(params: &'a Params, rows: usize, cols: usize) -> PolyMatrixNTT<'a> {
        let num_coeffs = rows * cols * params.poly_len * params.crt_count;
        let data: Vec<u64> = vec![0; num_coeffs];
        PolyMatrixNTT {
            params,
            rows,
            cols,
            data,
        }
    }
    fn random(params: &'a Params, rows: usize, cols: usize) -> Self {
        let rng = rand::thread_rng();
        let mut iter = rng.sample_iter(&Standard);
        let mut out = PolyMatrixNTT::zero(params, rows, cols);
        for r in 0..rows {
            for c in 0..cols {
                for i in 0..params.crt_count {
                    for j in 0..params.poly_len {
                        let idx = calc_index(&[i, j], &[params.crt_count, params.poly_len]);
                        let val: u64 = iter.next().unwrap();
                        out.get_poly_mut(r, c)[idx] = val % params.moduli[i];
                    }
                }
            }
        }
        out
    }
    fn pad_top(&self, pad_rows: usize) -> Self {
        let mut padded = Self::zero(self.params, self.rows + pad_rows, self.cols);
        padded.copy_into(&self, pad_rows, 0);
        padded
    }
}

impl<'a> PolyMatrixNTT<'a> {
    pub fn raw(&self) -> PolyMatrixRaw<'a> {
        from_ntt_alloc(&self)
    }
}

pub fn multiply_poly(params: &Params, res: &mut [u64], a: &[u64], b: &[u64]) {
    for c in 0..params.crt_count {
        for i in 0..params.poly_len {
            res[i] = multiply_modular(params, a[i], b[i], c);
        }
    }
}

pub fn multiply_add_poly(params: &Params, res: &mut [u64], a: &[u64], b: &[u64]) {
    for c in 0..params.crt_count {
        for i in 0..params.poly_len {
            res[i] = multiply_add_modular(params, a[i], b[i], res[i], c);
        }
    }
}

pub fn add_poly(params: &Params, res: &mut [u64], a: &[u64], b: &[u64]) {
    for c in 0..params.crt_count {
        for i in 0..params.poly_len {
            res[i] = add_modular(params, a[i], b[i], c);
        }
    }
}

pub fn invert_poly(params: &Params, res: &mut [u64], a: &[u64]) {
    for c in 0..params.crt_count {
        for i in 0..params.poly_len {
            res[i] = invert_modular(params, a[i], c);
        }
    }
}

pub fn automorph_poly(params: &Params, res: &mut [u64], a: &[u64], t: usize) {
    let poly_len = params.poly_len;
    for i in 0..poly_len {
        let num = (i * t) / poly_len;
        let rem = (i * t) % poly_len;

        if num % 2 == 0 {
            res[rem] = a[i];
        } else {
            res[rem] = params.modulus - a[i];
        }
    }
}

pub fn multiply_add_poly_avx(params: &Params, res: &mut [u64], a: &[u64], b: &[u64]) {
    for c in 0..params.crt_count {
        for i in (0..params.poly_len).step_by(4) {
            unsafe {
                let p_x = &a[c*params.poly_len + i] as *const u64;
                let p_y = &b[c*params.poly_len + i] as *const u64;
                let p_z = &mut res[c*params.poly_len + i] as *mut u64;
                let x = _mm256_loadu_si256(p_x as *const __m256i);
                let y = _mm256_loadu_si256(p_y as *const __m256i);
                let z = _mm256_loadu_si256(p_z as *const __m256i);

                let product = _mm256_mul_epu32(x, y);
                let out = _mm256_add_epi64(z, product);
                
                _mm256_storeu_si256(p_z as *mut __m256i, out);
            }
        }
    }
}

pub fn modular_reduce(params: &Params, res: &mut [u64]) {
    for c in 0..params.crt_count {
        for i in 0..params.poly_len {
            res[c*params.poly_len + i] %= params.moduli[c];
        }
    }
}

#[cfg(not(target_feature = "avx2"))]
pub fn multiply(res: &mut PolyMatrixNTT, a: &PolyMatrixNTT, b: &PolyMatrixNTT) {
    assert!(a.cols == b.rows);

    for i in 0..a.rows {
        for j in 0..b.cols {
            for z in 0..res.params.poly_len {
                res.get_poly_mut(i, j)[z] = 0;
            }
            for k in 0..a.cols {
                let params = res.params;
                let res_poly = res.get_poly_mut(i, j);
                let pol1 = a.get_poly(i, k);
                let pol2 = b.get_poly(k, j);
                multiply_add_poly(params, res_poly, pol1, pol2);
            }
        }
    }
}

#[cfg(target_feature = "avx2")]
pub fn multiply(res: &mut PolyMatrixNTT, a: &PolyMatrixNTT, b: &PolyMatrixNTT) {
    assert!(res.rows == a.rows);
    assert!(res.cols == b.cols);
    assert!(a.cols == b.rows);

    let params = res.params;
    for i in 0..a.rows {
        for j in 0..b.cols {
            for z in 0..res.params.poly_len {
                res.get_poly_mut(i, j)[z] = 0;
            }
            let res_poly = res.get_poly_mut(i, j);
            for k in 0..a.cols {
                let pol1 = a.get_poly(i, k);
                let pol2 = b.get_poly(k, j);
                multiply_add_poly_avx(params, res_poly, pol1, pol2);
            }
            modular_reduce(params, res_poly);
        }
    }
}

pub fn add(res: &mut PolyMatrixNTT, a: &PolyMatrixNTT, b: &PolyMatrixNTT) {
    assert!(res.rows == a.rows);
    assert!(res.cols == a.cols);
    assert!(a.rows == b.rows);
    assert!(a.cols == b.cols);

    let params = res.params;
    for i in 0..a.rows {
        for j in 0..a.cols {
            let res_poly = res.get_poly_mut(i, j);
            let pol1 = a.get_poly(i, j);
            let pol2 = b.get_poly(i, j);
            add_poly(params, res_poly, pol1, pol2);
        }
    }
}

pub fn invert(res: &mut PolyMatrixRaw, a: &PolyMatrixRaw) {
    assert!(res.rows == a.rows);
    assert!(res.cols == a.cols);

    let params = res.params;
    for i in 0..a.rows {
        for j in 0..a.cols {
            let res_poly = res.get_poly_mut(i, j);
            let pol1 = a.get_poly(i, j);
            invert_poly(params, res_poly, pol1);
        }
    }
}

pub fn automorph<'a>(res: &mut PolyMatrixRaw<'a>, a: &PolyMatrixRaw<'a>, t: usize) {
    assert!(res.rows == a.rows);
    assert!(res.cols == a.cols);

    let params = res.params;
    for i in 0..a.rows {
        for j in 0..a.cols {
            let res_poly = res.get_poly_mut(i, j);
            let pol1 = a.get_poly(i, j);
            automorph_poly(params, res_poly, pol1, t);
        }
    }
}

pub fn automorph_alloc<'a>(a: &PolyMatrixRaw<'a>, t: usize) -> PolyMatrixRaw<'a> {
    let mut res = PolyMatrixRaw::zero(a.params, a.rows, a.cols);
    automorph(&mut res, a, t);
    res
}

pub fn stack<'a>(a: &PolyMatrixRaw<'a>, b: &PolyMatrixRaw<'a>) -> PolyMatrixRaw<'a> {
    assert_eq!(a.cols, b.cols);
    let mut c = PolyMatrixRaw::zero(a.params, a.rows + b.rows, a.cols);
    c.copy_into(a, 0, 0);
    c.copy_into(b, a.rows, 0);
    c
}

pub fn scalar_multiply(res: &mut PolyMatrixNTT, a: &PolyMatrixNTT, b: &PolyMatrixNTT) {
    assert_eq!(a.rows, 1);
    assert_eq!(a.cols, 1);

    let params = res.params;
    let pol2 = a.get_poly(0, 0);
    for i in 0..b.rows {
        for j in 0..b.cols {
            let res_poly = res.get_poly_mut(i, j);
            let pol1 = b.get_poly(i, j);
            multiply_poly(params, res_poly, pol1, pol2);
        }
    }
}

pub fn scalar_multiply_alloc<'a>(a: &PolyMatrixNTT<'a>, b: &PolyMatrixNTT<'a>) -> PolyMatrixNTT<'a> {
    let mut res = PolyMatrixNTT::zero(b.params, b.rows, b.cols);
    scalar_multiply(&mut res, a, b);
    res
}

pub fn single_poly<'a>(params: &'a Params, val: u64) -> PolyMatrixRaw<'a> {
    let mut res = PolyMatrixRaw::zero(params, 1, 1);
    res.get_poly_mut(0, 0)[0] = val;
    res
}


pub fn to_ntt(a: &mut PolyMatrixNTT, b: &PolyMatrixRaw) {
    let params = a.params;
    for r in 0..a.rows {
        for c in 0..a.cols {
            let pol_src = b.get_poly(r, c);
            let pol_dst = a.get_poly_mut(r, c);
            for n in 0..params.crt_count {
                for z in 0..params.poly_len {
                    pol_dst[n * params.poly_len + z] = pol_src[z] % params.moduli[n];
                }
            }
            ntt_forward(params, pol_dst);
        }
    }
}

pub fn to_ntt_alloc<'a>(b: &PolyMatrixRaw<'a>) -> PolyMatrixNTT<'a> {
    let mut a = PolyMatrixNTT::zero(b.params, b.rows, b.cols);
    to_ntt(&mut a, b);
    a
}

pub fn from_ntt(a: &mut PolyMatrixRaw, b: &PolyMatrixNTT) {
    let params = a.params;
    SCRATCH.with(|scratch_cell| {
        let scratch_vec = &mut *scratch_cell.borrow_mut();
        let scratch = scratch_vec.as_mut_slice();
        for r in 0..a.rows {
            for c in 0..a.cols {
                let pol_src = b.get_poly(r, c);
                let pol_dst = a.get_poly_mut(r, c);
                scratch[0..pol_src.len()].copy_from_slice(pol_src);
                ntt_inverse(params, scratch);
                for z in 0..params.poly_len {
                    pol_dst[z] = params.crt_compose_2(scratch[z], scratch[params.poly_len + z]);
                }
            }
        }
    });
}

pub fn from_ntt_alloc<'a>(b: &PolyMatrixNTT<'a>) -> PolyMatrixRaw<'a> {
    let mut a = PolyMatrixRaw::zero(b.params, b.rows, b.cols);
    from_ntt(&mut a, b);
    a
}

impl<'a, 'b> Neg for &'b PolyMatrixRaw<'a> {
    type Output = PolyMatrixRaw<'a>;

    fn neg(self) -> Self::Output {
        let mut out = PolyMatrixRaw::zero(self.params, self.rows, self.cols);
        invert(&mut out, self);
        out
    }
}

impl<'a, 'b> Mul for &'b PolyMatrixNTT<'a> {
    type Output = PolyMatrixNTT<'a>;

    fn mul(self, rhs: Self) -> Self::Output {
        let mut out = PolyMatrixNTT::zero(self.params, self.rows, rhs.cols);
        multiply(&mut out, self, rhs);
        out
    }
}

impl<'a, 'b> Add for &'b PolyMatrixNTT<'a> {
    type Output = PolyMatrixNTT<'a>;

    fn add(self, rhs: Self) -> Self::Output {
        let mut out = PolyMatrixNTT::zero(self.params, self.rows, self.cols);
        add(&mut out, self, rhs);
        out
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn get_params() -> Params {
        get_test_params()
    }

    fn assert_all_zero(a: &[u64]) {
        for i in a {
            assert_eq!(*i, 0);
        }
    }

    #[test]
    fn sets_all_zeros() {
        let params = get_params();
        let m1 = PolyMatrixNTT::zero(&params, 2, 1);
        assert_all_zero(m1.as_slice());
    }

    #[test]
    fn multiply_correctness() {
        let params = get_params();
        let m1 = PolyMatrixNTT::zero(&params, 2, 1);
        let m2 = PolyMatrixNTT::zero(&params, 3, 2);
        let m3 = &m2 * &m1;
        assert_all_zero(m3.as_slice());
    }

    #[test]
    fn full_multiply_correctness() {
        let params = get_params();
        let mut m1 = PolyMatrixRaw::zero(&params, 1, 1);
        let mut m2 = PolyMatrixRaw::zero(&params, 1, 1);
        m1.get_poly_mut(0, 0)[1] = 100;
        m2.get_poly_mut(0, 0)[1] = 7;
        let m1_ntt = to_ntt_alloc(&m1);
        let m2_ntt = to_ntt_alloc(&m2);
        let m3_ntt = &m1_ntt * &m2_ntt;
        let m3 = from_ntt_alloc(&m3_ntt);
        assert_eq!(m3.get_poly(0, 0)[2], 700);
    }
}
