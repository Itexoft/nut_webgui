use std::fmt::Display;

use askama::FastWritable;

#[derive(Debug, Clone, Copy)]
pub enum SemanticType {
  None,
  Info,
  Error,
  Warning,
  Success,
}

impl Display for SemanticType {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      SemanticType::None => Ok(()),
      SemanticType::Info => f.write_str("Info"),
      SemanticType::Error => f.write_str("Error"),
      SemanticType::Warning => f.write_str("Warning"),
      SemanticType::Success => f.write_str("Success"),
    }
  }
}

impl FastWritable for SemanticType {
  fn write_into<W: core::fmt::Write + ?Sized>(
    &self,
    dest: &mut W,
    _: &dyn askama::Values,
  ) -> askama::Result<()> {
    match self {
      SemanticType::None => Ok(()),
      SemanticType::Info => dest.write_str("Info"),
      SemanticType::Error => dest.write_str("Error"),
      SemanticType::Warning => dest.write_str("Warning"),
      SemanticType::Success => dest.write_str("Success"),
    }?;

    Ok(())
  }
}

impl SemanticType {
  #[inline]
  pub fn as_badge(self) -> &'static str {
    BadgeStyle::from_type(self)
  }

  #[inline]
  pub fn as_text(self) -> &'static str {
    TextStyle::from_type(self)
  }

  #[inline]
  pub fn as_fill(self) -> &'static str {
    FillStyle::from_type(self)
  }

  #[inline]
  pub fn as_progress(self) -> &'static str {
    ProgressStyle::from_type(self)
  }

  #[inline]
  pub fn as_alert(self) -> &'static str {
    AlertStyle::from_type(self)
  }

  #[inline]
  pub fn as_input(self) -> &'static str {
    InputStyle::from_type(self)
  }

  #[inline]
  pub fn as_select(self) -> &'static str {
    SelectStyle::from_type(self)
  }
}

pub trait SemanticClass {
  fn error() -> &'static str;
  fn info() -> &'static str;
  fn success() -> &'static str;
  fn warning() -> &'static str;
  fn from_type(value: SemanticType) -> &'static str;
}

impl SemanticType {
  #[inline]
  pub fn from_range<T>(value: T, from: T, to: T) -> Self
  where
    T: PartialOrd + PartialEq,
  {
    let level = (value >= from) as u8 + (value >= to) as u8;

    match level {
      0 => SemanticType::Success,
      1 => SemanticType::Warning,
      2 => SemanticType::Error,
      _ => unreachable!(),
    }
  }

  #[inline]
  pub fn from_range_inverted<T>(value: T, from: T, to: T) -> Self
  where
    T: PartialOrd + PartialEq,
  {
    let level = (value >= from) as u8 + (value >= to) as u8;

    match level {
      0 => SemanticType::Error,
      1 => SemanticType::Warning,
      2 => SemanticType::Success,
      _ => unreachable!(),
    }
  }
}

macro_rules! impl_semantic_class {
  ($struct_name:ident, { info = $info:literal, success = $success:literal, warning = $warning:literal, error = $error:literal }) => {
    impl SemanticClass for $struct_name {
      #[inline]
      fn info() -> &'static str {
        $info
      }

      #[inline]
      fn warning() -> &'static str {
        $warning
      }

      #[inline]
      fn success() -> &'static str {
        $success
      }

      #[inline]
      fn error() -> &'static str {
        $error
      }

      #[inline]
      fn from_type(value: SemanticType) -> &'static str {
        match value {
          SemanticType::None => "",
          SemanticType::Info => $info,
          SemanticType::Error => $error,
          SemanticType::Warning => $warning,
          SemanticType::Success => $success,
        }
      }
    }
  };
}

pub struct TextStyle;
pub struct BadgeStyle;
pub struct FillStyle;
pub struct ProgressStyle;
pub struct AlertStyle;
pub struct InputStyle;
pub struct SelectStyle;

impl_semantic_class!(
  TextStyle,
  {
    info = "text-info",
    success = "text-success",
    warning = "text-warning",
    error = "text-error"
  }
);

impl_semantic_class!(
  BadgeStyle,
  {
    info = "badge-info",
    success = "badge-success",
    warning = "badge-warning",
    error = "badge-error"
  }

);

impl_semantic_class!(
  FillStyle,
  {
    info = "fill-info",
    success = "fill-success",
    warning = "fill-warning",
    error = "fill-error"
  }
);

impl_semantic_class!(
  ProgressStyle,
  {
    info = "progress-info",
    success = "progress-success",
    warning = "progress-warning",
    error = "progress-error"
  }
);

impl_semantic_class!(
  AlertStyle,
  {
    info = "alert-info",
    success = "alert-success",
    warning = "alert-warning",
    error = "alert-error"
  }
);

impl_semantic_class!(
  InputStyle,
  {
    info = "input-info",
    success = "input-success",
    warning = "input-warning",
    error = "input-error"
  }
);

impl_semantic_class!(
  SelectStyle,
  {
    info = "select-info",
    success = "select-success",
    warning = "select-warning",
    error = "select-error"
  }
);
