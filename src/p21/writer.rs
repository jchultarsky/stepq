//! Writer for the Part 21 exchange structure.
//!
//! Output is assembled from the source text. Every instance is copied
//! byte for byte from its original span — whitespace, comments, number
//! formatting and string escapes included — and only instance names
//! (`#id` tokens) are rewritten when renumbering. A string literal is a
//! single token, so `#` inside one is never touched. Comments inside an
//! instance are copied as they are, so a comment that mentions an old
//! `#id` keeps the old number.

use std::io::{BufWriter, Write};

use super::exchange::{Exchange, Instance};
use super::lexer::TokenKind;
use crate::error::{Error, Result};

/// How instance names are assigned in the output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Numbering {
    /// Keep every instance's original `#id`. Each written instance is then
    /// identical to its source text.
    #[default]
    Preserve,
    /// Number the written instances `#1`, `#2`, … in file order.
    Dense,
}

/// Writes all or part of an [`Exchange`] back out as Part 21.
///
/// ```
/// use stepq::p21::{Numbering, Writer, parse};
///
/// let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;\
///             #10=A(#30,'see #30');#20=B();#30=C();ENDSEC;END-ISO-10303-21;";
/// let exchange = parse(src)?;
/// let mut out = Vec::new();
/// // Write #10 and #30 only, renumbered.
/// Writer::new(&exchange)
///     .numbering(Numbering::Dense)
///     .write_selection([0, 2], &mut out)?;
/// let text = String::from_utf8(out).unwrap();
/// assert!(text.contains("DATA;\n#1=A(#2,'see #30');\n#2=C();\nENDSEC;\n"));
/// # Ok::<(), stepq::Error>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Writer<'e, 'a> {
    exchange: &'e Exchange<'a>,
    header: Option<&'e [u8]>,
    numbering: Numbering,
}

impl<'e, 'a> Writer<'e, 'a> {
    /// A writer for `exchange` that copies the source header and keeps
    /// instance names.
    pub fn new(exchange: &'e Exchange<'a>) -> Self {
        Self {
            exchange,
            header: None,
            numbering: Numbering::Preserve,
        }
    }

    /// Chooses how instance names are assigned.
    #[must_use]
    pub fn numbering(self, numbering: Numbering) -> Self {
        Self { numbering, ..self }
    }

    /// Replaces the header entities.
    ///
    /// `entities` is written between `HEADER;` and `ENDSEC;` exactly as
    /// given, so it must be complete header records, each ending in `;`.
    /// By default the source header is copied verbatim.
    #[must_use]
    pub fn header(self, entities: &'e [u8]) -> Self {
        Self {
            header: Some(entities),
            ..self
        }
    }

    /// Writes every instance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnresolvedReference`] if the source itself has a
    /// dangling reference (nothing is written), and [`Error::Io`] if
    /// writing fails.
    pub fn write_all<W: Write>(&self, out: W) -> Result<()> {
        self.write_selection(0..self.exchange.instances().len(), out)
    }

    /// Writes the instances at `positions` — indices into
    /// [`Exchange::instances`] — once each and in file order, whatever
    /// order `positions` lists them in.
    ///
    /// Data sections are kept, with their parameters, even when none of
    /// their instances is selected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnresolvedReference`] if a selected instance refers
    /// to one that is not selected. That is checked before anything is
    /// written: a file with a dangling reference is exactly the silent
    /// failure `docs/ARCHITECTURE.md` warns about. Returns [`Error::Io`] if
    /// writing fails.
    ///
    /// # Panics
    ///
    /// Panics if a position is out of range.
    pub fn write_selection<W: Write>(
        &self,
        positions: impl IntoIterator<Item = usize>,
        out: W,
    ) -> Result<()> {
        let names = self.assign_names(positions);
        self.check_closed(&names)?;
        let mut out = BufWriter::new(out);
        self.write_file(&mut out, &names)?;
        out.flush()?;
        Ok(())
    }

    /// The output name of every instance, by position; `None` if the
    /// instance is not selected.
    fn assign_names(&self, positions: impl IntoIterator<Item = usize>) -> Vec<Option<u64>> {
        let instances = self.exchange.instances();
        let mut names = vec![None; instances.len()];
        for position in positions {
            names[position] = Some(instances[position].id);
        }
        if self.numbering == Numbering::Dense {
            let mut next = 0;
            for name in names.iter_mut().flatten() {
                next += 1;
                *name = next;
            }
        }
        names
    }

    fn check_closed(&self, names: &[Option<u64>]) -> Result<()> {
        let exchange = self.exchange;
        for (instance, name) in exchange.instances().iter().zip(names) {
            if name.is_none() {
                continue;
            }
            for id in exchange.references(instance) {
                target_name(exchange, names, instance, id)?;
            }
        }
        Ok(())
    }

    fn write_file(&self, out: &mut impl Write, names: &[Option<u64>]) -> Result<()> {
        let exchange = self.exchange;
        out.write_all(b"ISO-10303-21;\nHEADER;")?;
        out.write_all(self.header.unwrap_or_else(|| exchange.header_text()))?;
        out.write_all(b"ENDSEC;\n")?;
        for (params, positions) in exchange.data_sections() {
            out.write_all(b"DATA")?;
            out.write_all(params)?;
            out.write_all(b";\n")?;
            for position in positions {
                if let Some(name) = names[position] {
                    self.write_instance(out, &exchange.instances()[position], name, names)?;
                    out.write_all(b"\n")?;
                }
            }
            out.write_all(b"ENDSEC;\n")?;
        }
        out.write_all(b"END-ISO-10303-21;\n")?;
        Ok(())
    }

    fn write_instance(
        &self,
        out: &mut impl Write,
        instance: &Instance,
        name: u64,
        names: &[Option<u64>],
    ) -> Result<()> {
        let exchange = self.exchange;
        let text = exchange.text(instance);
        if self.numbering == Numbering::Preserve {
            out.write_all(text)?;
            return Ok(());
        }

        let src = exchange.source();
        let digits = text
            .get(1..)
            .unwrap_or_default()
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        write!(out, "#{name}")?;
        let mut cursor = instance.span.start + 1 + digits;
        for token in exchange.instance_tokens(instance) {
            if let TokenKind::InstanceName(id) = token.kind {
                let target = target_name(exchange, names, instance, id)?;
                out.write_all(&src[cursor..token.span.start])?;
                write!(out, "#{target}")?;
                cursor = token.span.end;
            }
        }
        out.write_all(&src[cursor..instance.span.end])?;
        Ok(())
    }
}

/// The output name of the instance `from` refers to as `#id`.
fn target_name(
    exchange: &Exchange<'_>,
    names: &[Option<u64>],
    from: &Instance,
    id: u64,
) -> Result<u64> {
    exchange
        .position(id)
        .and_then(|position| names[position])
        .ok_or(Error::UnresolvedReference {
            from: from.id,
            to: id,
        })
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::p21::parse;

    fn file_with(data: &str) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('X'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    fn write(src: &str, numbering: Numbering, positions: Option<&[usize]>) -> String {
        let exchange = parse(src.as_bytes()).unwrap();
        let writer = Writer::new(&exchange).numbering(numbering);
        let mut out = Vec::new();
        match positions {
            Some(positions) => writer.write_selection(positions.iter().copied(), &mut out),
            None => writer.write_all(&mut out),
        }
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn preserve_reproduces_the_source() {
        let src = file_with(
            "#1=A('x',#2);\n#2 = B ( /* keep */ 1.50E+01, '\\X2\\00E9\\X0\\' ) ;\n#3=(C()D(#1));\n",
        );
        assert_eq!(write(&src, Numbering::Preserve, None), src);
    }

    #[test]
    fn dense_renumbering_rewrites_only_instance_names() {
        let src = file_with(
            "#10=A(#30,'see #30');\n#20=B();\n#30=C(#10) /* was #10 */;\n#007=D((#20,#20));\n",
        );
        let expected =
            file_with("#1=A(#3,'see #30');\n#2=B();\n#3=C(#1) /* was #10 */;\n#4=D((#2,#2));\n");
        assert_eq!(write(&src, Numbering::Dense, None), expected);
    }

    #[test]
    fn selections_are_written_in_file_order() {
        let src = file_with("#10=A(#30,'see #30');\n#20=B();\n#30=C(#10);\n");
        assert_eq!(
            write(&src, Numbering::Dense, Some(&[2, 0, 2])),
            file_with("#1=A(#2,'see #30');\n#2=C(#1);\n")
        );
        assert_eq!(
            write(&src, Numbering::Preserve, Some(&[2, 0])),
            file_with("#10=A(#30,'see #30');\n#30=C(#10);\n")
        );
    }

    #[test]
    fn open_selections_are_rejected_before_writing() {
        let src = file_with("#10=A(#30);\n#20=B();\n#30=C();\n");
        let exchange = parse(src.as_bytes()).unwrap();
        let mut out = Vec::new();
        let err = Writer::new(&exchange)
            .write_selection([1, 0], &mut out)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::UnresolvedReference { from: 10, to: 30 }
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn header_can_be_replaced() {
        let src = file_with("#1=A();\n");
        let exchange = parse(src.as_bytes()).unwrap();
        let mut out = Vec::new();
        Writer::new(&exchange)
            .header(b"\nFILE_SCHEMA(('Y'));\n")
            .write_all(&mut out)
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            src.replace("('X')", "('Y')")
        );
    }

    #[test]
    fn sections_keep_their_parameters() {
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('A','B'));ENDSEC;\
                   ANCHOR;<origin>=#1;ENDSEC;\
                   DATA ( 'one' , ('A') );#1=X();ENDSEC;\
                   DATA('two',('B'));#2=Y(#1);#3=Z();ENDSEC;\
                   END-ISO-10303-21;";
        assert_eq!(
            write(src, Numbering::Dense, Some(&[2])),
            "ISO-10303-21;\nHEADER;FILE_SCHEMA(('A','B'));ENDSEC;\n\
             DATA( 'one' , ('A') );\nENDSEC;\n\
             DATA('two',('B'));\n#1=Z();\nENDSEC;\n\
             END-ISO-10303-21;\n"
        );
    }

    #[test]
    fn io_errors_are_reported() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let src = file_with("#1=A();\n");
        let exchange = parse(src.as_bytes()).unwrap();
        assert!(matches!(
            Writer::new(&exchange).write_all(Full),
            Err(Error::Io(_))
        ));
    }
}
