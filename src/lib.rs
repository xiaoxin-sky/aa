use napi_derive::napi;

#[napi]
pub fn sum(a: u32, b: u32) -> u32 {
  a + b
}
