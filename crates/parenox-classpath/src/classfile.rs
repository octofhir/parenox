//! Read the bounded metadata needed by Java completion without loading classes.
use std::collections::BTreeSet;

const ACC_PUBLIC: u16 = 0x0001;
const ACC_STATIC: u16 = 0x0008;
const ACC_FINAL: u16 = 0x0010;
const ACC_BRIDGE: u16 = 0x0040;
const ACC_INTERFACE: u16 = 0x0200;
const ACC_SYNTHETIC: u16 = 0x1000;
const ACC_MODULE: u16 = 0x8000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum JavaMemberKind {
    Method,
    Field,
    Constant,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct JavaMember {
    pub name: String,
    pub kind: JavaMemberKind,
    pub descriptor: String,
}

impl JavaMember {
    pub(super) fn parameter_types(&self) -> Option<&str> {
        if self.kind != JavaMemberKind::Method {
            return None;
        }
        self.descriptor.split_once(')').map(|(params, _)| params)
    }

    /// Describe the JVM signature using Java type spellings.
    pub fn detail(&self) -> String {
        let method = self.kind == JavaMemberKind::Method;
        let mut parameters = Vec::new();
        let detail = descriptor_types(self.descriptor.as_bytes(), method, |ty| {
            parameters.push(java_type(ty)?);
            Some(())
        })
        .and_then(java_type);
        match detail {
            Some(result) if method => format!("{result} ({})", parameters.join(", ")),
            Some(result) => result,
            None => self.descriptor.clone(),
        }
    }
}

pub(super) struct Class {
    /// JVM internal names use a slash between package components.
    pub name: String,
    pub superclass: Option<String>,
    pub interfaces: Vec<String>,
    pub public: bool,
    pub static_members: Vec<JavaMember>,
    pub instance_methods: Vec<JavaMember>,
    pub bridges: Vec<JavaMember>,
}

fn class_header(bytes: &[u8]) -> Option<(Input<'_>, Pool<'_>, u16, String)> {
    let mut input = Input(bytes);
    if input.u32()? != 0xcafe_babe {
        return None;
    }
    input.u16()?; // minor version
    if input.u16()? < 45 {
        return None;
    }
    let pool = Pool::read(&mut input)?;
    let access = input.u16()?;
    let name = pool.class_name(input.u16()?)?;
    Some((input, pool, access, name))
}

pub(super) fn is_public_class(bytes: &[u8], expected: &str) -> bool {
    class_header(bytes).is_some_and(|(_, _, access, name)| {
        access & (ACC_PUBLIC | ACC_SYNTHETIC | ACC_MODULE) == ACC_PUBLIC && name == expected
    })
}

pub(super) fn read_class(bytes: &[u8]) -> Option<Class> {
    let (mut input, pool, access, name) = class_header(bytes)?;
    let superclass = match input.u16()? {
        0 if name == "java/lang/Object" || access & ACC_MODULE != 0 => None,
        0 => return None,
        index => Some(pool.class_name(index)?),
    };
    let interfaces = (0..input.u16()?)
        .map(|_| pool.class_name(input.u16()?))
        .collect::<Option<Vec<_>>>()?;
    let mut members = Members::default();
    read_members(&mut input, &pool, false, &mut members)?;
    read_members(&mut input, &pool, true, &mut members)?;
    attributes(&mut input, &pool)?;
    input.0.is_empty().then(|| Class {
        name,
        superclass: if access & ACC_INTERFACE == 0 {
            superclass
        } else {
            None
        },
        interfaces,
        public: access & ACC_PUBLIC != 0,
        static_members: members.static_members.into_iter().collect(),
        instance_methods: members.instance_methods.into_iter().collect(),
        bridges: members.bridges.into_iter().collect(),
    })
}

#[derive(Default)]
struct Members {
    static_members: BTreeSet<JavaMember>,
    instance_methods: BTreeSet<JavaMember>,
    bridges: BTreeSet<JavaMember>,
}

fn read_members(
    input: &mut Input<'_>,
    pool: &Pool<'_>,
    methods: bool,
    members: &mut Members,
) -> Option<()> {
    for _ in 0..input.u16()? {
        let access = input.u16()?;
        let name = pool.utf8(input.u16()?)?;
        let descriptor = pool.utf8(input.u16()?)?;
        let synthetic = attributes(input, pool)?;
        descriptor_types(descriptor, methods, |_| Some(()))?;
        let static_member = access & ACC_STATIC != 0;
        let bridge = methods && !static_member && access & ACC_BRIDGE != 0;
        if access & ACC_PUBLIC == 0
            || ((access & ACC_SYNTHETIC != 0 || synthetic) && !bridge)
            || (!static_member && !methods)
            || matches!(name, b"<init>" | b"<clinit>")
        {
            continue;
        }
        let name = decode_modified_utf8(name)?;
        if name.is_empty()
            || name.contains(['.', ';', '[', '/', '\0'])
            || (methods && name.contains(['<', '>']))
        {
            return None;
        }
        let kind = if methods {
            JavaMemberKind::Method
        } else if access & ACC_FINAL != 0 {
            JavaMemberKind::Constant
        } else {
            JavaMemberKind::Field
        };
        let members = if bridge {
            &mut members.bridges
        } else if static_member {
            &mut members.static_members
        } else {
            &mut members.instance_methods
        };
        members.insert(JavaMember {
            name,
            kind,
            descriptor: decode_modified_utf8(descriptor)?,
        });
    }
    Some(())
}

/// Attributes are opaque payloads here, including bytecode and annotations.
fn attributes(input: &mut Input<'_>, pool: &Pool<'_>) -> Option<bool> {
    let mut synthetic = false;
    for _ in 0..input.u16()? {
        let name = pool.utf8(input.u16()?)?;
        let length = usize::try_from(input.u32()?).ok()?;
        input.take(length)?;
        if name == b"Synthetic" {
            if length != 0 {
                return None;
            }
            synthetic = true;
        }
    }
    Some(synthetic)
}

#[derive(Clone, Copy)]
enum Constant<'a> {
    Unusable,
    Utf8(&'a [u8]),
    Class(u16),
    Other,
}

struct Pool<'a>(Vec<Constant<'a>>);

impl<'a> Pool<'a> {
    fn read(input: &mut Input<'a>) -> Option<Self> {
        let count = usize::from(input.u16()?);
        if count == 0 || count - 1 > input.0.len() / 3 {
            return None;
        }
        let mut constants = Vec::with_capacity(count);
        constants.push(Constant::Unusable);
        while constants.len() < count {
            let tag = input.u8()?;
            let entry = match tag {
                1 => {
                    let length = usize::from(input.u16()?);
                    let bytes = input.take(length)?;
                    if !valid_modified_utf8(bytes) {
                        return None;
                    }
                    Constant::Utf8(bytes)
                }
                7 => Constant::Class(input.u16()?),
                3 | 4 | 9..=12 | 17 | 18 => {
                    input.take(4)?;
                    Constant::Other
                }
                5 | 6 => {
                    // A wide constant reserves the following pool index.
                    if constants.len() + 1 >= count {
                        return None;
                    }
                    input.take(8)?;
                    constants.push(Constant::Other);
                    Constant::Unusable
                }
                8 | 16 | 19 | 20 => {
                    input.take(2)?;
                    Constant::Other
                }
                15 => {
                    input.take(3)?;
                    Constant::Other
                }
                _ => return None,
            };
            constants.push(entry);
        }
        Some(Self(constants))
    }

    fn utf8(&self, index: u16) -> Option<&'a [u8]> {
        match self.0.get(usize::from(index))? {
            Constant::Utf8(bytes) => Some(bytes),
            _ => None,
        }
    }

    fn class(&self, index: u16) -> Option<&'a [u8]> {
        match self.0.get(usize::from(index))? {
            Constant::Class(name) => self.utf8(*name),
            _ => None,
        }
    }

    fn class_name(&self, index: u16) -> Option<String> {
        let raw = self.class(index)?;
        if !valid_internal_name(raw) {
            return None;
        }
        let name = decode_modified_utf8(raw)?;
        (!name.contains('\0')).then_some(name)
    }
}

struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn take(&mut self, size: usize) -> Option<&'a [u8]> {
        let (value, rest) = self.0.split_at_checked(size)?;
        self.0 = rest;
        Some(value)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
}

fn valid_internal_name(bytes: &[u8]) -> bool {
    !bytes.contains(&b'.')
        && !bytes.contains(&b';')
        && !bytes.contains(&b'[')
        && bytes
            .split(|&byte| byte == b'/')
            .all(|part| !part.is_empty())
}

fn descriptor_types<'a>(
    bytes: &'a [u8],
    method: bool,
    mut parameter: impl FnMut(&'a [u8]) -> Option<()>,
) -> Option<&'a [u8]> {
    let mut descriptor = Input(bytes);
    if method {
        if descriptor.u8() != Some(b'(') {
            return None;
        }
        while descriptor.0.first().is_some_and(|&byte| byte != b')') {
            parameter(field_type(&mut descriptor)?)?;
        }
        if descriptor.u8() != Some(b')') {
            return None;
        }
        if descriptor.0 == b"V" {
            return Some(descriptor.0);
        }
    }
    let result = field_type(&mut descriptor)?;
    descriptor.0.is_empty().then_some(result)
}

fn field_type<'a>(descriptor: &mut Input<'a>) -> Option<&'a [u8]> {
    let before = descriptor.0;
    let depth = array_depth(before);
    if depth > 255 {
        return None;
    }
    descriptor.take(depth)?;
    match descriptor.u8()? {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' => {}
        b'L' => {
            let length = descriptor.0.iter().position(|&byte| byte == b';')?;
            let name = descriptor.take(length)?;
            if !valid_internal_name(name) {
                return None;
            }
            descriptor.take(1)?;
        }
        _ => return None,
    }
    Some(&before[..before.len() - descriptor.0.len()])
}

fn array_depth(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|&&byte| byte == b'[').count()
}

fn java_type(bytes: &[u8]) -> Option<String> {
    let depth = array_depth(bytes);
    let mut name = match &bytes[depth..] {
        b"B" => "byte",
        b"C" => "char",
        b"D" => "double",
        b"F" => "float",
        b"I" => "int",
        b"J" => "long",
        b"S" => "short",
        b"Z" => "boolean",
        b"V" => "void",
        [b'L', name @ .., b';'] => {
            let name = std::str::from_utf8(name).ok()?.replace('/', ".");
            return Some(name + &"[]".repeat(depth));
        }
        _ => return None,
    }
    .to_owned();
    name.push_str(&"[]".repeat(depth));
    Some(name)
}

fn valid_modified_utf8(bytes: &[u8]) -> bool {
    let mut input = Input(bytes);
    while !input.0.is_empty() {
        if utf16_unit(&mut input).is_none() {
            return false;
        }
    }
    true
}

fn decode_modified_utf8(bytes: &[u8]) -> Option<String> {
    // The constant pool already passed encoding validation. Multibyte
    // supplementary characters and NUL need the modified encoding path.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_owned());
    }
    let mut input = Input(bytes);
    let mut units = Vec::new();
    while !input.0.is_empty() {
        units.push(utf16_unit(&mut input)?);
    }
    String::from_utf16(&units).ok()
}

fn utf16_unit(input: &mut Input<'_>) -> Option<u16> {
    let first = input.u8()?;
    let continuation = |input: &mut Input<'_>| {
        let byte = input.u8()?;
        (byte & 0xc0 == 0x80).then_some(u16::from(byte & 0x3f))
    };
    match first {
        1..=0x7f => Some(first.into()),
        0xc0..=0xdf => {
            let unit = (u16::from(first & 0x1f) << 6) | continuation(input)?;
            (unit == 0 || unit >= 0x80).then_some(unit)
        }
        0xe0..=0xef => {
            let unit =
                (u16::from(first & 0xf) << 12) | (continuation(input)? << 6) | continuation(input)?;
            (unit >= 0x800).then_some(unit)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fixture {
        pool: Vec<u8>,
        slots: u16,
    }

    impl Fixture {
        fn constant(&mut self, tag: u8, bytes: &[u8], slots: u16) -> u16 {
            let index = self.slots + 1;
            self.pool.push(tag);
            self.pool.extend_from_slice(bytes);
            self.slots += slots;
            index
        }

        fn utf8(&mut self, bytes: &[u8]) -> u16 {
            let mut entry = (bytes.len() as u16).to_be_bytes().to_vec();
            entry.extend_from_slice(bytes);
            self.constant(1, &entry, 1)
        }

        fn class(&mut self, name: &[u8]) -> u16 {
            let name = self.utf8(name);
            self.constant(7, &name.to_be_bytes(), 1)
        }

        fn finish(self, access: u16, class: u16, superclass: u16) -> Vec<u8> {
            let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 61];
            bytes.extend_from_slice(&(self.slots + 1).to_be_bytes());
            bytes.extend(self.pool);
            words(&mut bytes, &[access, class, superclass, 0]);
            bytes
        }
    }

    fn words(bytes: &mut Vec<u8>, values: &[u16]) {
        bytes.extend(values.iter().flat_map(|value| value.to_be_bytes()));
    }

    fn sample_class() -> Vec<u8> {
        let mut fixture = Fixture::default();
        let superclass = fixture.class(b"java/lang/Object");
        fixture.constant(5, &[0; 8], 2);
        fixture.constant(6, &[0; 8], 2);
        // An unused string literal may have modified UTF-8 that Rust strings
        // cannot represent directly, including a lone UTF-16 surrogate.
        fixture.utf8(b"\xc0\x80\xed\xa0\x80");
        let class = fixture.class(b"sample/Owner");
        let names = [
            fixture.utf8(b"DEFAULT"),
            fixture.utf8(b"make"),
            fixture.utf8(b"hidden"),
            fixture.utf8(b"instance"),
            fixture.utf8(b"synthetic"),
            fixture.utf8(b"\xed\xa0\x81\xed\xb0\x80"),
        ];
        let field_type = fixture.utf8(b"Ljava/lang/String;");
        let method_type = fixture.utf8(b"()[Ljava/lang/String;");
        let overload_type = fixture.utf8(b"([[I)Ljava/lang/String;");
        let synthetic_attribute = fixture.utf8(b"Synthetic");
        let mut bytes = fixture.finish(ACC_PUBLIC, class, superclass);
        let exposed = ACC_PUBLIC | ACC_STATIC;
        words(
            &mut bytes,
            &[
                5, // field count
                exposed | ACC_FINAL,
                names[0],
                field_type,
                0,
                exposed,
                names[1],
                field_type,
                0,
                ACC_STATIC,
                names[2],
                field_type,
                0,
                ACC_PUBLIC,
                names[3],
                field_type,
                0,
                exposed | ACC_SYNTHETIC,
                names[4],
                field_type,
                0,
                4, // method count
                exposed,
                names[1],
                method_type,
                0,
                exposed,
                names[1],
                overload_type,
                0,
                exposed,
                names[5],
                method_type,
                0,
                exposed,
                names[4],
                method_type,
                1,
                synthetic_attribute,
                0,
                0, // attribute name and length
                0, // class attribute count
            ],
        );
        bytes
    }

    #[test]
    fn exposes_public_static_members_and_preserves_overloads() {
        let class = read_class(&sample_class()).unwrap();
        assert_eq!(class.name, "sample/Owner");
        assert_eq!(class.superclass.as_deref(), Some("java/lang/Object"));
        assert!(class.public);
        let members: Vec<_> = class
            .static_members
            .iter()
            .map(|member| (member.name.as_str(), member.kind, member.detail()))
            .collect();
        assert_eq!(
            members,
            [
                (
                    "DEFAULT",
                    JavaMemberKind::Constant,
                    "java.lang.String".into()
                ),
                (
                    "make",
                    JavaMemberKind::Method,
                    "java.lang.String[] ()".into()
                ),
                (
                    "make",
                    JavaMemberKind::Method,
                    "java.lang.String (int[][])".into()
                ),
                ("make", JavaMemberKind::Field, "java.lang.String".into()),
                ("𐐀", JavaMemberKind::Method, "java.lang.String[] ()".into()),
            ]
        );
        assert_eq!(
            class.static_members[2].descriptor,
            "([[I)Ljava/lang/String;"
        );
    }

    #[test]
    fn describes_void_and_primitive_signatures() {
        for (descriptor, detail) in [
            ("(JJ)J", "long (long, long)"),
            (
                "([Ljava/lang/Object;Z)V",
                "void (java.lang.Object[], boolean)",
            ),
        ] {
            let member = JavaMember {
                name: "example".into(),
                kind: JavaMemberKind::Method,
                descriptor: descriptor.into(),
            };
            assert_eq!(member.detail(), detail);
        }
    }

    #[test]
    fn rejects_truncated_or_malformed_classfiles() {
        let bytes = sample_class();
        for length in 0..bytes.len() {
            assert!(read_class(&bytes[..length]).is_none(), "length {length}");
        }
        let mut malformed = bytes.clone();
        malformed.push(0);
        assert!(read_class(&malformed).is_none());

        let mut malformed = bytes;
        malformed[10] = 2; // Unassigned constant tag at pool index 1.
        assert!(read_class(&malformed).is_none());
        let mut standalone_pool = Input(&[0, 2, 5, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(Pool::read(&mut standalone_pool).is_none());
    }

    #[test]
    fn recognizes_the_root_class_and_ordinary_package_access() {
        let mut fixture = Fixture::default();
        let class = fixture.class(b"java/lang/Object");
        let mut bytes = fixture.finish(0, class, 0);
        words(&mut bytes, &[0, 0, 0]);
        let root = read_class(&bytes).unwrap();
        assert!(!root.public);
        assert_eq!(root.superclass, None);
    }
}
