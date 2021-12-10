use dprint_core::formatting::tokens::{TokenCollection, TokenFinder as CoreTokenFinder};
use jsonc_parser::common::Ranged;
use jsonc_parser::tokens::{Token, TokenAndRange};

pub struct TokenFinder<'a> {
  inner: CoreTokenFinder<LocalTokenCollection<'a>>,
}

impl<'a> TokenFinder<'a> {
  pub fn new(tokens: &'a Vec<TokenAndRange<'a>>) -> TokenFinder<'a> {
    TokenFinder {
      inner: CoreTokenFinder::new(LocalTokenCollection(tokens)),
    }
  }

  pub fn get_next_token_if_comma(&mut self, node: &dyn Ranged) -> Option<&'a TokenAndRange<'a>> {
    self.inner.get_next_token_if(node.end(), |token| token.token == Token::Comma)
  }

  #[inline]
  pub fn get_previous_token(&mut self, node: &dyn Ranged) -> Option<&'a TokenAndRange<'a>> {
    self.inner.get_previous_token(node.start())
  }
}

// Wrap and implement a trait for the CoreTokenFinder

struct LocalTokenCollection<'a>(&'a Vec<TokenAndRange<'a>>);

impl<'a> TokenCollection<'a> for LocalTokenCollection<'a> {
  type TPos = usize;
  type TToken = TokenAndRange<'a>;
  fn get_start_at_index(&self, index: usize) -> usize {
    self.0[index].range.start
  }
  fn get_end_at_index(&self, index: usize) -> usize {
    self.0[index].range.end
  }
  fn get_token_at_index(&self, index: usize) -> &'a TokenAndRange {
    &self.0[index]
  }
  fn len(&self) -> usize {
    self.0.len()
  }
  fn is_empty(&self) -> bool {
    self.0.is_empty()
  }
}
