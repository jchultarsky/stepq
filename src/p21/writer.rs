//! Writer for the Part 21 exchange structure.
//!
//! Output is assembled from the source text. Every instance is copied
//! byte for byte from its original span — whitespace, comments, number
//! formatting and string escapes included — and only instance names
//! (`#id` tokens) are rewritten when renumbering. A string literal is a
//! single token, so `#` inside one is never touched. Comments inside an
//! instance are copied as they are, so a comment that mentions an old
//! `#id` keeps the old number.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufWriter, Write};

use super::exchange::{Exchange, ExtraKind, ExtraSection, Instance, ReferenceName};
use super::lexer::{Span, Token, TokenKind};
use crate::error::{Error, Result};

/// Text to write in place of individual tokens, such as a string literal
/// blanked or a name replaced. Every token without a replacement is still
/// copied from the source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replacements {
    /// By the token's start offset in the source: its end, and the text.
    by_start: BTreeMap<usize, (usize, Vec<u8>)>,
}

impl Replacements {
    /// No replacements.
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes `text` instead of the token at `span`, which must be exactly
    /// one token's span, as from [`Literal::span`](super::Literal::span).
    /// `text` is written as given, so a string must include its quotes. A
    /// later replacement of the same token wins.
    pub fn replace(&mut self, span: Span, text: impl Into<Vec<u8>>) {
        self.by_start.insert(span.start, (span.end, text.into()));
    }

    /// The number of tokens replaced.
    pub fn len(&self) -> usize {
        self.by_start.len()
    }

    /// True if nothing is replaced.
    pub fn is_empty(&self) -> bool {
        self.by_start.is_empty()
    }

    fn get(&self, span: Span) -> Option<&[u8]> {
        self.by_start
            .get(&span.start)
            .filter(|(end, _)| *end == span.end)
            .map(|(_, text)| text.as_slice())
    }

    fn any_within(&self, span: Span) -> bool {
        self.by_start.range(span.start..span.end).next().is_some()
    }
}

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
    replacements: Option<&'e Replacements>,
}

impl<'e, 'a> Writer<'e, 'a> {
    /// A writer for `exchange` that copies the source header and keeps
    /// instance names.
    pub fn new(exchange: &'e Exchange<'a>) -> Self {
        Self {
            exchange,
            header: None,
            numbering: Numbering::Preserve,
            replacements: None,
        }
    }

    /// Writes replacement text for individual tokens of the source header
    /// and of instances. Instances with no replaced token are still copied
    /// byte for byte; a header given with [`header`](Self::header) is
    /// written as given.
    #[must_use]
    pub fn replacements(self, replacements: &'e Replacements) -> Self {
        Self {
            replacements: Some(replacements),
            ..self
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
        self.write_pruned(positions, [], out)
    }

    /// Like [`write_selection`](Self::write_selection), except that the
    /// instances at `pruned` positions may list instances that are not
    /// selected: those list items are removed, with their separating commas,
    /// and everything else in the instance is copied as usual.
    ///
    /// This is how an aggregate shared by many products — a layer
    /// assignment, a category — is written into a file holding only some of
    /// them. Only items of a list are removed; a pruned instance's other
    /// references must still be selected. A list can become empty, which the
    /// caller must avoid where the schema forbids it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnresolvedReference`] for a reference to an
    /// unselected instance that is not a list item of a pruned instance,
    /// before anything is written; [`Error::Io`] if writing fails.
    ///
    /// # Panics
    ///
    /// Panics if a position is out of range.
    pub fn write_pruned<W: Write>(
        &self,
        positions: impl IntoIterator<Item = usize>,
        pruned: impl IntoIterator<Item = usize>,
        out: W,
    ) -> Result<()> {
        let names = self.assign_names(positions);
        let mut is_pruned = vec![false; names.data.len()];
        for position in pruned {
            is_pruned[position] = true;
        }
        let plans = self.check_closed(&names, &is_pruned)?;
        let mut out = BufWriter::new(out);
        self.write_file(&mut out, &names, &plans)?;
        out.flush()?;
        Ok(())
    }

    /// The output name of every selected instance, and of every external
    /// instance name (edition 3) a selected instance refers to. When
    /// renumbering, external names follow the instances, in the order of
    /// their `REFERENCE` entries.
    fn assign_names(&self, positions: impl IntoIterator<Item = usize>) -> Names {
        let exchange = self.exchange;
        let instances = exchange.instances();
        let mut data = vec![None; instances.len()];
        for position in positions {
            data[position] = Some(instances[position].id);
        }
        let used: HashSet<u64> = instances
            .iter()
            .zip(&data)
            .filter(|(_, name)| name.is_some())
            .flat_map(|(instance, _)| exchange.references(instance))
            .filter(|&id| exchange.is_external(id))
            .collect();
        let external_ids = exchange
            .external_references()
            .into_iter()
            .filter_map(|reference| match reference.name {
                ReferenceName::Instance(id) if used.contains(&id) => Some(id),
                _ => None,
            });

        let mut external = HashMap::new();
        if self.numbering == Numbering::Dense {
            let mut next = 0;
            for name in data.iter_mut().flatten() {
                next += 1;
                *name = next;
            }
            for id in external_ids {
                external.entry(id).or_insert_with(|| {
                    next += 1;
                    next
                });
            }
        } else {
            external.extend(external_ids.map(|id| (id, id)));
        }
        Names { data, external }
    }

    /// Checks that every selected instance's references resolve, and plans
    /// the list items to remove from pruned instances. Returns, by position,
    /// which of a pruned instance's tokens to drop.
    fn check_closed(&self, names: &Names, pruned: &[bool]) -> Result<Vec<Option<Vec<bool>>>> {
        let exchange = self.exchange;
        let mut plans = vec![None; names.data.len()];
        for (position, (instance, name)) in exchange.instances().iter().zip(&names.data).enumerate()
        {
            if name.is_none() {
                continue;
            }
            if pruned[position] {
                plans[position] = Some(self.removal_plan(instance, names)?);
            } else {
                for id in exchange.references(instance) {
                    target_name(exchange, names, instance, id)?;
                }
            }
        }
        Ok(plans)
    }

    /// Which tokens of `instance` to drop so that no list item refers to an
    /// unselected instance, keeping exactly one comma between the items left.
    fn removal_plan(&self, instance: &Instance, names: &Names) -> Result<Vec<bool>> {
        let exchange = self.exchange;
        let tokens = exchange.instance_tokens(instance);

        // The `(` enclosing each token; a `(` right after a name opens a
        // record or typed value, not a list.
        let mut enclosing = vec![None; tokens.len()];
        let mut open = Vec::new();
        for (i, token) in tokens.iter().enumerate() {
            enclosing[i] = open.last().copied();
            match token.kind {
                TokenKind::LeftParen => open.push(i),
                TokenKind::RightParen => {
                    open.pop();
                }
                _ => {}
            }
        }
        let in_list = |i: usize| {
            enclosing[i].is_some_and(|paren: usize| {
                paren > 0
                    && !matches!(
                        tokens[paren - 1].kind,
                        TokenKind::Keyword | TokenKind::UserKeyword
                    )
            })
        };

        let mut removed = vec![false; tokens.len()];
        for (i, token) in tokens.iter().enumerate() {
            let TokenKind::InstanceName(id) = token.kind else {
                continue;
            };
            if target_name(exchange, names, instance, id).is_ok() {
                continue;
            }
            let before = i.checked_sub(1).map(|j| tokens[j].kind);
            let after = tokens.get(i + 1).map(|t| t.kind);
            let is_item = in_list(i)
                && matches!(before, Some(TokenKind::LeftParen | TokenKind::Comma))
                && matches!(after, Some(TokenKind::Comma | TokenKind::RightParen));
            if !is_item {
                return Err(Error::UnresolvedReference {
                    from: instance.id,
                    to: id,
                });
            }
            removed[i] = true;
            if after == Some(TokenKind::Comma) {
                removed[i + 1] = true;
            } else {
                // The last item: drop the nearest comma before it still kept.
                let mut j = i;
                while j > 0 {
                    j -= 1;
                    if removed[j] {
                        continue;
                    }
                    if tokens[j].kind == TokenKind::Comma {
                        removed[j] = true;
                    }
                    break;
                }
            }
        }
        Ok(removed)
    }

    fn write_file(
        &self,
        out: &mut impl Write,
        names: &Names,
        plans: &[Option<Vec<bool>>],
    ) -> Result<()> {
        let exchange = self.exchange;
        // Unchanged output keeps edition 3 sections as written, signatures
        // included; anything else invalidates a signature.
        let verbatim = self.numbering == Numbering::Preserve
            && self.replacements.is_none()
            && names.data.iter().all(Option::is_some)
            && plans.iter().all(Option::is_none);
        let extra = exchange.extra_sections();
        out.write_all(b"ISO-10303-21;\nHEADER;")?;
        match (self.header, self.replacements) {
            (Some(header), _) => out.write_all(header)?,
            (None, Some(replacements)) if replacements.any_within(exchange.header_span()) => {
                write_replaced(
                    out,
                    exchange.source(),
                    exchange.header_span(),
                    exchange.header_tokens(),
                    replacements,
                )?;
            }
            (None, _) => out.write_all(exchange.header_text())?,
        }
        out.write_all(b"ENDSEC;\n")?;
        for section in extra.iter().filter(|s| s.kind != ExtraKind::Signature) {
            if verbatim {
                out.write_all(section.span.slice(exchange.source()))?;
                out.write_all(b"\n")?;
            } else {
                self.write_extra_section(out, section, names)?;
            }
        }
        for (params, positions) in exchange.data_sections() {
            out.write_all(b"DATA")?;
            out.write_all(params)?;
            out.write_all(b";\n")?;
            for position in positions {
                if let Some(name) = names.data[position] {
                    self.write_instance(
                        out,
                        &exchange.instances()[position],
                        name,
                        names,
                        plans[position].as_deref(),
                    )?;
                    out.write_all(b"\n")?;
                }
            }
            out.write_all(b"ENDSEC;\n")?;
        }
        if verbatim {
            for section in extra.iter().filter(|s| s.kind == ExtraKind::Signature) {
                out.write_all(section.span.slice(exchange.source()))?;
                out.write_all(b"\n")?;
            }
        }
        out.write_all(b"END-ISO-10303-21;\n")?;
        Ok(())
    }

    /// Writes the entries of an `ANCHOR` or `REFERENCE` section that still
    /// apply, renamed: anchors whose instances are all written, and
    /// references a written instance uses (constant and value names always).
    fn write_extra_section(
        &self,
        out: &mut impl Write,
        section: &ExtraSection,
        names: &Names,
    ) -> Result<()> {
        let exchange = self.exchange;
        let src = exchange.source();
        let kept: Vec<_> = section
            .entries
            .iter()
            .map(|(span, range)| (*span, exchange.tokens_in(range.clone())))
            .filter(
                |(_, tokens)| match (section.kind, tokens.first().map(|t| t.kind)) {
                    (ExtraKind::Reference, Some(TokenKind::InstanceName(id))) => {
                        names.external.contains_key(&id)
                    }
                    (ExtraKind::Reference, _) => true,
                    _ => tokens.iter().all(|token| match token.kind {
                        TokenKind::InstanceName(id) => names.of(exchange, id).is_some(),
                        _ => true,
                    }),
                },
            )
            .collect();
        if kept.is_empty() {
            return Ok(());
        }
        let keyword: &[u8] = if section.kind == ExtraKind::Anchor {
            b"ANCHOR;\n"
        } else {
            b"REFERENCE;\n"
        };
        out.write_all(keyword)?;
        for (span, tokens) in kept {
            let mut cursor = span.start;
            for token in tokens {
                if let TokenKind::InstanceName(id) = token.kind {
                    if let Some(name) = names.of(exchange, id).filter(|&name| name != id) {
                        out.write_all(&src[cursor..token.span.start])?;
                        write!(out, "#{name}")?;
                        cursor = token.span.end;
                    }
                }
            }
            out.write_all(&src[cursor..span.end])?;
            out.write_all(b"\n")?;
        }
        out.write_all(b"ENDSEC;\n")?;
        Ok(())
    }

    /// Writes one instance from its source text, renaming instance names
    /// when renumbering and dropping the tokens `removed` marks.
    fn write_instance(
        &self,
        out: &mut impl Write,
        instance: &Instance,
        name: u64,
        names: &Names,
        removed: Option<&[bool]>,
    ) -> Result<()> {
        let exchange = self.exchange;
        let text = exchange.text(instance);
        let dense = self.numbering == Numbering::Dense;
        let replacements = self
            .replacements
            .filter(|replacements| replacements.any_within(instance.span));
        if !dense && removed.is_none() && replacements.is_none() {
            out.write_all(text)?;
            return Ok(());
        }

        let src = exchange.source();
        let mut cursor = instance.span.start;
        if dense {
            let digits = text
                .get(1..)
                .unwrap_or_default()
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            write!(out, "#{name}")?;
            cursor += 1 + digits;
        }
        for (i, token) in exchange.instance_tokens(instance).iter().enumerate() {
            if removed.is_some_and(|removed| removed[i]) {
                out.write_all(&src[cursor..token.span.start])?;
                cursor = token.span.end;
            } else if let Some(text) = replacements.and_then(|r| r.get(token.span)) {
                out.write_all(&src[cursor..token.span.start])?;
                out.write_all(text)?;
                cursor = token.span.end;
            } else if let (true, TokenKind::InstanceName(id)) = (dense, token.kind) {
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

/// Writes `span` of `src`, with the replaced tokens among `tokens` swapped
/// for their replacement text.
fn write_replaced(
    out: &mut impl Write,
    src: &[u8],
    span: Span,
    tokens: &[Token],
    replacements: &Replacements,
) -> std::io::Result<()> {
    let mut cursor = span.start;
    for token in tokens {
        if token.span.start < cursor || token.span.end > span.end {
            continue;
        }
        if let Some(text) = replacements.get(token.span) {
            out.write_all(&src[cursor..token.span.start])?;
            out.write_all(text)?;
            cursor = token.span.end;
        }
    }
    out.write_all(&src[cursor..span.end])
}

/// Output names: of every instance by position (`None` if not written), and
/// of the external instance names that are written.
struct Names {
    data: Vec<Option<u64>>,
    external: HashMap<u64, u64>,
}

impl Names {
    /// The output name of `#id`, if it is written.
    fn of(&self, exchange: &Exchange<'_>, id: u64) -> Option<u64> {
        match exchange.position(id) {
            Some(position) => self.data[position],
            None => self.external.get(&id).copied(),
        }
    }
}

/// The output name of the instance `from` refers to as `#id`.
fn target_name(exchange: &Exchange<'_>, names: &Names, from: &Instance, id: u64) -> Result<u64> {
    names.of(exchange, id).ok_or(Error::UnresolvedReference {
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

    #[test]
    fn edition_3_sections_follow_renumbering_and_selection() {
        let src = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('X'));\nENDSEC;\n\
                   ANCHOR;<origin>=#10;<tip>=#9;ENDSEC;\n\
                   REFERENCE;#9=<bolt.stp#shape>;#LIB=<lib.stp#x>;ENDSEC;\n\
                   DATA;\n#10=X(#20,#9);\n#20=Y();\nENDSEC;\n\
                   SIGNATURE;'c2ln';ENDSEC;\n\
                   END-ISO-10303-21;\n";
        // Unchanged: every section as written, the signature included.
        assert_eq!(write(src, Numbering::Preserve, None), src);

        assert_eq!(
            write(src, Numbering::Dense, None),
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('X'));\nENDSEC;\n\
             ANCHOR;\n<origin>=#1;\n<tip>=#3;\nENDSEC;\n\
             REFERENCE;\n#3=<bolt.stp#shape>;\n#LIB=<lib.stp#x>;\nENDSEC;\n\
             DATA;\n#1=X(#2,#3);\n#2=Y();\nENDSEC;\n\
             END-ISO-10303-21;\n"
        );

        // Only #20: neither anchor nor #9 applies any more.
        assert_eq!(
            write(src, Numbering::Preserve, Some(&[1])),
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('X'));\nENDSEC;\n\
             REFERENCE;\n#LIB=<lib.stp#x>;\nENDSEC;\n\
             DATA;\n#20=Y();\nENDSEC;\n\
             END-ISO-10303-21;\n"
        );
    }

    #[test]
    fn replacements_change_only_their_tokens() {
        let src = "ISO-10303-21;\nHEADER;\nFILE_NAME('a.stp','',('Jane'),('ACME'),'','','');\nFILE_SCHEMA(('X'));\nENDSEC;\nDATA;\n#10=A('secret',#20,'keep');\n#20=B('also secret');\n#30=C('untouched',  1.50);\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(src.as_bytes()).unwrap();
        let mut replacements = Replacements::new();
        let author = exchange
            .header_entity("FILE_NAME")
            .unwrap()
            .param(2)
            .unwrap();
        let author = author.list().unwrap().next().unwrap().literal().unwrap();
        replacements.replace(author.span(), "''");
        let records = |position: usize| {
            exchange
                .records(&exchange.instances()[position])
                .next()
                .unwrap()
        };
        let secret = records(0).param(0).unwrap().literal().unwrap();
        replacements.replace(secret.span(), "'x'");
        let also = records(1).param(0).unwrap().literal().unwrap();
        replacements.replace(also.span(), "''");
        assert_eq!(replacements.len(), 3);

        let mut out = Vec::new();
        Writer::new(&exchange)
            .replacements(&replacements)
            .write_all(&mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("FILE_NAME('a.stp','',(''),('ACME'),'','','');"),
            "{text}"
        );
        assert!(
            text.contains("#10=A('x',#20,'keep');\n#20=B('');\n#30=C('untouched',  1.50);\n"),
            "{text}"
        );

        let mut out = Vec::new();
        Writer::new(&exchange)
            .replacements(&replacements)
            .numbering(Numbering::Dense)
            .write_all(&mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("#1=A('x',#2,'keep');\n#2=B('');\n#3=C('untouched',  1.50);\n"),
            "{text}"
        );
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

    /// Positions 0–2 are #1–#3; 3–7 are the lists. #2 is left out.
    const LISTS: &str = "#1=A();\n#2=B();\n#3=C();\n\
         #10=L('middle',(#1, #2 ,#3));\n\
         #11=L('only',(#2));\n\
         #12=L('last and repeated',(#1,#2,#3,#2));\n\
         #13=L('nested',((#2,#1),#3));\n\
         #14=L('all',(#2,#2));\n";

    fn write_pruned(numbering: Numbering) -> String {
        let src = file_with(LISTS);
        let exchange = parse(src.as_bytes()).unwrap();
        let mut out = Vec::new();
        Writer::new(&exchange)
            .numbering(numbering)
            .write_pruned([0, 2, 3, 4, 5, 6, 7], [3, 4, 5, 6, 7], &mut out)
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn pruning_removes_list_items_and_their_commas() {
        assert_eq!(
            write_pruned(Numbering::Preserve),
            file_with(
                "#1=A();\n#3=C();\n\
                 #10=L('middle',(#1,  #3));\n\
                 #11=L('only',());\n\
                 #12=L('last and repeated',(#1,#3));\n\
                 #13=L('nested',((#1),#3));\n\
                 #14=L('all',());\n"
            )
        );
        assert_eq!(
            write_pruned(Numbering::Dense),
            file_with(
                "#1=A();\n#2=C();\n\
                 #3=L('middle',(#1,  #2));\n\
                 #4=L('only',());\n\
                 #5=L('last and repeated',(#1,#2));\n\
                 #6=L('nested',((#1),#2));\n\
                 #7=L('all',());\n"
            )
        );
    }

    #[test]
    fn pruning_never_removes_attributes_that_are_not_list_items() {
        for data in [
            "#1=A();\n#2=R(#1);\n",
            "#1=A();\n#2=R('x',#1);\n",
            "#1=A();\n#2=R(T(#1));\n",
            "#1=A();\n#2=(P(#1)Q());\n",
        ] {
            let src = file_with(data);
            let exchange = parse(src.as_bytes()).unwrap();
            let mut out = Vec::new();
            let err = Writer::new(&exchange)
                .write_pruned([1], [1], &mut out)
                .unwrap_err();
            assert!(
                matches!(err, Error::UnresolvedReference { from: 2, to: 1 }),
                "{data}"
            );
            assert!(out.is_empty(), "{data}");
        }
    }

    #[test]
    fn unpruned_instances_still_reject_open_selections() {
        let src = file_with(LISTS);
        let exchange = parse(src.as_bytes()).unwrap();
        let mut out = Vec::new();
        let err = Writer::new(&exchange)
            .write_pruned([0, 2, 3], [], &mut out)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::UnresolvedReference { from: 10, to: 2 }
        ));
    }
}
