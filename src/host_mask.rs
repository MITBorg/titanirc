use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::{Display, Formatter},
    io::{Error, ErrorKind},
    iter::once,
    str::FromStr,
};

use itertools::Either;
use sqlx::{
    database::{HasArguments, HasValueRef},
    encode::IsNull,
    error::BoxDynError,
    Database, Decode, Encode, Type,
};

/// A map of `HostMask`s to `T`, implemented as a prefix trie with three
/// sections with support for wildcards.
#[derive(Debug)]
pub struct HostMaskMap<T> {
    children: HashMap<Key, Node<T>>,
    matcher: Matcher,
}

impl<T> HostMaskMap<T> {
    /// Instantiates a new `HostMaskMap` with a top level capacity of 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            matcher: Matcher::Nick,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (String, &T)> {
        self.iter_inner(String::new(), self.matcher)
    }

    fn iter_inner(&self, s: String, last_seen: Matcher) -> impl Iterator<Item = (String, &T)> {
        self.children
            .iter()
            .flat_map(move |(k, v)| {
                let (k, next_matcher) = match k {
                    Key::Wildcard => (
                        format!("{s}*{}", last_seen.splitter()),
                        last_seen.next().unwrap_or(last_seen),
                    ),
                    Key::EndOfString => (
                        format!("{s}{}", last_seen.splitter()),
                        last_seen.next().unwrap_or(last_seen),
                    ),
                    Key::Char(c) => (format!("{s}{c}"), last_seen),
                };

                match v {
                    Node::Match(v) => Either::Left(once((k, v))),
                    Node::Inner(v) => Either::Right(v.iter_inner(k, next_matcher)),
                }
            })
            // TODO
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Inserts a new mask into the tree with the given `value`. This function operates
    /// in `O(m)` average time complexity
    pub fn insert(&mut self, mask: &HostMask<'_>, value: T) {
        let mut next_mask = mask.as_borrowed();

        let key = match self.matcher {
            Matcher::Nick => take_next_char(&mask.nick, &mut next_mask.nick),
            Matcher::Username => take_next_char(&mask.username, &mut next_mask.username),
            Matcher::Host => take_next_char(&mask.host, &mut next_mask.host),
        };

        let key = match key {
            Some('*') => Key::Wildcard,
            Some(c) => Key::Char(c),
            None => Key::EndOfString,
        };

        if key.is_end() && self.matcher.next().is_none() {
            self.children.insert(key, Node::Match(value));
        } else {
            let node = self.children.entry(key).or_insert_with(|| {
                Node::Inner(Self {
                    children: HashMap::new(),
                    matcher: if key.is_end() {
                        self.matcher.next().expect("guarded by if")
                    } else {
                        self.matcher
                    },
                })
            });

            match node {
                Node::Match(_) => unreachable!("stored hostmask has less parts than a!b@c"),
                Node::Inner(map) => map.insert(&next_mask, value),
            };
        }
    }

    /// Fetches all the matches within the trie that match the input. This function returns
    /// any exact matches as well as any wildcard matches. This function operates in `O(m)`
    /// average time complexity.
    #[must_use]
    pub fn get(&self, mask: &HostMask<'_>) -> Vec<&T> {
        self.get_inner(mask, Vec::new())
    }

    fn get_inner<'a>(&'a self, mask: &HostMask<'_>, mut out: Vec<&'a T>) -> Vec<&'a T> {
        let mut next_mask = mask.as_borrowed();

        let key = match self.matcher {
            Matcher::Nick => take_next_char(&mask.nick, &mut next_mask.nick),
            Matcher::Username => take_next_char(&mask.username, &mut next_mask.username),
            Matcher::Host => take_next_char(&mask.host, &mut next_mask.host),
        };

        if let Some(next) = self.children.get(&key.map_or(Key::EndOfString, Key::Char)) {
            out = traverse(out, next, &next_mask);
        }

        if let Some(wildcard) = self.children.get(&Key::Wildcard) {
            out = traverse(out, wildcard, &next_mask);
        }

        out
    }
}

impl<'a, T> FromIterator<(HostMask<'a>, T)> for HostMaskMap<T> {
    fn from_iter<I: IntoIterator<Item = (HostMask<'a>, T)>>(iter: I) -> Self {
        let mut out = Self::new();

        for (k, v) in iter {
            out.insert(&k, v);
        }

        out
    }
}

impl<T> Default for HostMaskMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Traverses the trie, appending any matches into `out` before returning.
fn traverse<'a, T>(mut out: Vec<&'a T>, node: &'a Node<T>, mask: &HostMask<'_>) -> Vec<&'a T> {
    match node {
        Node::Match(v) => {
            out.push(v);
        }
        Node::Inner(child) => {
            out = child.get_inner(mask, out);
        }
    }

    out
}

/// Takes a single character from `v` and updates `next` to the remaining input.
fn take_next_char<'a>(v: &'a str, next: &mut Cow<'a, str>) -> Option<char> {
    let mut chars = v.chars();
    let c = chars.next();
    *next = Cow::Borrowed(chars.as_str());
    c
}

#[derive(Hash, PartialEq, Eq, Debug, Copy, Clone)]
enum Key {
    Wildcard,
    EndOfString,
    Char(char),
}

impl Key {
    const fn is_end(self) -> bool {
        !matches!(self, Self::Char(_))
    }
}

#[derive(Debug)]
enum Node<T> {
    Match(T),
    Inner(HostMaskMap<T>),
}

#[derive(Copy, Clone, Debug)]
enum Matcher {
    Nick,
    Username,
    Host,
}

impl Matcher {
    const fn next(self) -> Option<Self> {
        match self {
            Self::Nick => Some(Self::Username),
            Self::Username => Some(Self::Host),
            Self::Host => None,
        }
    }

    const fn splitter(self) -> &'static str {
        match self {
            Self::Nick => "!",
            Self::Username => "@",
            Self::Host => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HostMask<'a> {
    nick: Cow<'a, str>,
    username: Cow<'a, str>,
    host: Cow<'a, str>,
}

impl<'a> HostMask<'a> {
    #[must_use]
    pub const fn new(nick: &'a str, username: &'a str, host: &'a str) -> Self {
        Self {
            nick: Cow::Borrowed(nick),
            username: Cow::Borrowed(username),
            host: Cow::Borrowed(host),
        }
    }

    #[must_use]
    pub fn as_borrowed(&'a self) -> Self {
        Self {
            nick: Cow::Borrowed(self.nick.as_ref()),
            username: Cow::Borrowed(self.username.as_ref()),
            host: Cow::Borrowed(self.host.as_ref()),
        }
    }

    #[must_use]
    pub fn into_owned(self) -> HostMask<'static> {
        HostMask {
            nick: Cow::Owned(self.nick.into_owned()),
            username: Cow::Owned(self.username.into_owned()),
            host: Cow::Owned(self.host.into_owned()),
        }
    }
}

impl Display for HostMask<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}!{}@{}", self.nick, self.username, self.host)
    }
}

impl<'a, DB> Type<DB> for HostMask<'a>
where
    String: Type<DB>,
    DB: Database,
{
    fn type_info() -> DB::TypeInfo {
        String::type_info()
    }

    fn compatible(ty: &DB::TypeInfo) -> bool {
        String::compatible(ty)
    }
}

impl<'a, 'q, DB> Encode<'q, DB> for HostMask<'a>
where
    String: Encode<'q, DB>,
    DB: Database,
{
    fn encode_by_ref(&self, buf: &mut <DB as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
        self.to_string().encode(buf)
    }
}

impl<'r, DB> Decode<'r, DB> for HostMask<'static>
where
    &'r str: Decode<'r, DB>,
    DB: Database,
{
    fn decode(value: <DB as HasValueRef<'r>>::ValueRef) -> Result<Self, BoxDynError> {
        Ok(<&'r str as Decode<'r, DB>>::decode(value)?.parse()?)
    }
}

impl FromStr for HostMask<'static> {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        HostMask::try_from(s).map(HostMask::into_owned)
    }
}

impl<'a> TryFrom<&'a str> for HostMask<'a> {
    type Error = Error;

    fn try_from(rest: &'a str) -> Result<Self, Self::Error> {
        let (nick, rest) = rest.split_once('!').unwrap_or((rest, ""));
        let (username, host) = rest.split_once('@').unwrap_or(("*", "*"));

        let is_invalid = |v: &str| {
            (v.contains('*') && !v.ends_with('*'))
                || v.chars().filter(|&c| c == '*').count() > 1
                || v.is_empty()
        };

        if is_invalid(nick) {
            return Err(Error::new(ErrorKind::Other, "invalid nick"));
        } else if is_invalid(username) {
            return Err(Error::new(ErrorKind::Other, "invalid username"));
        } else if is_invalid(host) {
            return Err(Error::new(ErrorKind::Other, "invalid hostname"));
        }

        Ok(Self {
            nick: Cow::Borrowed(nick),
            username: Cow::Borrowed(username),
            host: Cow::Borrowed(host),
        })
    }
}

#[cfg(test)]
mod test {
    use crate::host_mask::{HostMask, HostMaskMap};

    #[test]
    fn wildcard_middle_of_string_unsupported() {
        assert!(HostMask::try_from("aa*a!bbbb@cccc").is_err());
    }

    #[test]
    fn multiple_wildcards_unsupported() {
        assert!(HostMask::try_from("a**!bbb@cccc").is_err());
    }

    #[test]
    fn empty_key_unsupported() {
        assert!(HostMask::try_from("a!@cccc").is_err());
    }

    #[test]
    fn test_insert_and_get_no_wildcard() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!bbbb@cccc".try_into().unwrap(), 10);

        let retrieved = map.get(&"aaaa!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 10);
    }

    #[test]
    fn test_insert_with_wildcard_and_get_exact() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!*@*".try_into().unwrap(), 20);

        let retrieved = map.get(&"aaaa!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 20);
    }

    #[test]
    fn test_insert_with_wildcard_and_get_wildcard() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!*@*".try_into().unwrap(), 30);

        let retrieved = map.get(&"aaaa!*@*".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 30);
    }

    #[test]
    fn test_insert_multiple_and_get_one() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!bbbb@cccc".try_into().unwrap(), 40);
        map.insert(&"xxxx!yyyy@zzzz".try_into().unwrap(), 50);

        let retrieved = map.get(&"aaaa!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 40);
    }

    #[test]
    fn test_insert_and_get_multiple_matches() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!*@*".try_into().unwrap(), 60);
        map.insert(&"*!bbbb@cccc".try_into().unwrap(), 70);

        let retrieved = map.get(&"aaaa!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved.contains(&&60));
        assert!(retrieved.contains(&&70));
    }

    #[test]
    fn test_get_no_match() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa!bbbb@cccc".try_into().unwrap(), 80);

        let retrieved = map.get(&"xxxx!yyyy@zzzz".try_into().unwrap());
        assert_eq!(retrieved.len(), 0);
    }

    #[test]
    fn test_insert_with_partial_wildcard_and_get_exact() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa*!bbbb@cccc".try_into().unwrap(), 100);

        let retrieved = map.get(&"aaaa1234!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 100);
    }

    #[test]
    fn test_insert_with_partial_wildcard_and_no_match() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa*!bbbb@cccc".try_into().unwrap(), 110);

        let retrieved = map.get(&"aaab!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 0);
    }

    #[test]
    fn test_insert_multiple_partial_wildcards_and_get_one() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa*!bbbb@cccc".try_into().unwrap(), 120);
        map.insert(&"xxxx*!yyyy@zzzz".try_into().unwrap(), 130);

        let retrieved = map.get(&"aaaa123!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 120);
    }

    #[test]
    fn test_insert_with_multiple_wildcard_styles_and_get_match() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa*!bbbb@cccc".try_into().unwrap(), 140);
        map.insert(&"xxxx!*@*".try_into().unwrap(), 150);

        let retrieved = map.get(&"aaaa123!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 1);
        assert_eq!(*retrieved[0], 140);

        let retrieved2 = map.get(&"xxxx!testyyyy@zzzz".try_into().unwrap());
        assert_eq!(retrieved2.len(), 1);
        assert_eq!(*retrieved2[0], 150);
    }

    #[test]
    fn test_insert_with_partial_wildcard_and_get_multiple_matches() {
        let mut map = HostMaskMap::new();
        map.insert(&"aaaa*!bbbb@cccc".try_into().unwrap(), 160);
        map.insert(&"aaaa*!bbbb@ccc*".try_into().unwrap(), 170);

        let retrieved = map.get(&"aaaa1234!bbbb@cccc".try_into().unwrap());
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved.contains(&&160));
        assert!(retrieved.contains(&&170));
    }
}
