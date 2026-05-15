use std::fmt;
use tracing::field::{Field, Visit};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::Writer;

const SENSITIVE: &[&str] = &[
    "path", "hash", "content", "exif", "metadata", "data", "artifact", "output", "input",
];

fn is_sensitive(name: &str) -> bool {
    SENSITIVE.iter().any(|&s| name.eq_ignore_ascii_case(s))
}

#[derive(Debug, Clone)]
pub struct RedactingFields;

impl Default for RedactingFields {
    fn default() -> Self {
        Self::new()
    }
}

impl RedactingFields {
    pub fn new() -> Self {
        Self
    }
}

impl<'writer> tracing_subscriber::fmt::FormatFields<'writer> for RedactingFields {
    fn format_fields<R: RecordFields>(
        &self,
        mut writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        let mut visitor = RedactVisitor::new(&mut writer);
        fields.record(&mut visitor);
        Ok(())
    }
}

struct RedactVisitor<'a> {
    writer: &'a mut dyn fmt::Write,
    first: bool,
}

impl<'a> RedactVisitor<'a> {
    fn new(writer: &'a mut dyn fmt::Write) -> Self {
        Self {
            writer,
            first: true,
        }
    }

    fn write_separator(&mut self) -> fmt::Result {
        if self.first {
            self.first = false;
        } else {
            self.writer.write_char(' ')?;
        }
        Ok(())
    }

    fn write_name(&mut self, field: &Field) -> fmt::Result {
        self.write_separator()?;
        write!(self.writer, "{}=", field.name())
    }

    fn write_redacted(&mut self, field: &Field) -> fmt::Result {
        self.write_name(field)?;
        self.writer.write_str("[REDACTED]")
    }

    fn write_value<T: fmt::Display>(&mut self, field: &Field, value: T) -> fmt::Result {
        if is_sensitive(field.name()) {
            self.write_redacted(field)
        } else {
            self.write_name(field)?;
            write!(self.writer, "{}", value)
        }
    }
}

impl Visit for RedactVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = self.write_value(field, format!("{:?}", value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = self.write_value(field, format!("{:?}", value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        let _ = self.write_value(field, value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        let _ = self.write_value(field, value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        let _ = self.write_value(field, value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let _ = self.write_value(field, value);
    }
}
