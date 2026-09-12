use std::collections::HashMap;

/// The order to write an object's properties in, keyed by the object's start position.
///
/// Generation writes an object's properties in source order unless it finds the object here, so an
/// empty map leaves the file's ordering alone. Only objects whose order actually changes get an
/// entry, which keeps everything a reordered object has to give up — the blank lines between its
/// properties — for the objects that are left as they were written.
#[derive(Debug, Default)]
pub struct PropertyOrders(HashMap<usize, Vec<usize>>);

impl PropertyOrders {
  pub fn new() -> Self {
    Default::default()
  }

  /// The order to write the properties of the object starting at `start`, or `None` to write them
  /// in the order they were parsed.
  pub fn get(&self, start: usize) -> Option<&[usize]> {
    self.0.get(&start).map(|order| order.as_slice())
  }

  pub fn insert(&mut self, start: usize, order: Vec<usize>) {
    self.0.insert(start, order);
  }

  #[cfg(test)]
  pub fn orders(&self) -> impl Iterator<Item = &Vec<usize>> {
    self.0.values()
  }
}
