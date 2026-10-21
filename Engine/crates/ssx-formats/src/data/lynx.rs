use crate::lynx_schema::{self, ParamType};
use crate::xml::{self, Element};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct ParticleSystem {
    pub version: Option<String>,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub name: String,
    pub params: Vec<Param>,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub params: Vec<Param>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub class_name: String,
    pub instance_name: Option<String>,
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub states: Vec<State>,
}

#[derive(Clone, Debug)]
pub struct State {
    pub id: String,
    pub params: Vec<Param>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub index: u32,
    pub num_elements: Option<u32>,
    pub raw: String,
    pub value: Value,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Float(f32),
    Vector(Vec<f32>),
    Reference(String),
    String(String),
}

impl Value {
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Value::Float(f) => Some(f),
            Value::Int(i) => Some(i as f32),
            _ => None,
        }
    }
    pub fn as_i32(&self) -> Option<i32> {
        match *self {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl ParticleSystem {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(data).map_err(|_| Error::invalid("lnx: not UTF-8"))?;
        let root = xml::parse(text)?;
        if root.name != "particleSystem" {
            return Err(Error::invalid(format!("lnx: root element {}", root.name)));
        }
        only_children(&root, &["particleEffect"])?;
        let effects = root.children_named("particleEffect").map(parse_effect).collect::<Result<_>>()?;
        Ok(ParticleSystem { version: root.attr("version").map(str::to_owned), effects })
    }

    pub fn effect(&self, name: &str) -> Option<&Effect> {
        self.effects.iter().find(|e| e.name == name)
    }
}

impl Effect {
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }
}

impl Group {
    pub fn action(&self, class_name: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.class_name == class_name)
    }
}

impl Action {
    pub fn param(&self, name: &str, index: u32) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name && p.index == index)
    }
    pub fn state(&self, id: &str) -> Option<&State> {
        self.states.iter().find(|s| s.id == id)
    }
}

fn required<'a>(el: &'a Element, attr: &str) -> Result<&'a str> {
    el.attr(attr).ok_or_else(|| Error::invalid(format!("lnx: <{}> without {attr}", el.name)))
}

fn only_children(el: &Element, allowed: &[&str]) -> Result<()> {
    match el.children.iter().find(|c| !allowed.contains(&c.name.as_str())) {
        Some(c) => Err(Error::invalid(format!("lnx: unexpected <{}> in <{}>", c.name, el.name))),
        None => Ok(()),
    }
}

fn parse_effect(el: &Element) -> Result<Effect> {
    only_children(el, &["parameter", "particleGroup"])?;
    let class = lynx_schema::class("ParticleEffect");
    Ok(Effect {
        name: required(el, "name")?.to_owned(),
        params: parse_params(el, class)?,
        groups: el.children_named("particleGroup").map(parse_group).collect::<Result<_>>()?,
    })
}

fn parse_group(el: &Element) -> Result<Group> {
    only_children(el, &["parameter", "particleAction"])?;
    let class = lynx_schema::class("ParticleGroup");
    Ok(Group {
        name: required(el, "name")?.to_owned(),
        params: parse_params(el, class)?,
        actions: el.children_named("particleAction").map(parse_action).collect::<Result<_>>()?,
    })
}

fn parse_action(el: &Element) -> Result<Action> {
    only_children(el, &["parameter", "state"])?;
    let class_name = required(el, "className")?.to_owned();
    let class = lynx_schema::class(&class_name);
    let mut states = Vec::new();
    for s in el.children_named("state") {
        only_children(s, &["parameter"])?;
        states.push(State { id: required(s, "id")?.to_owned(), params: parse_params(s, class)? });
    }
    Ok(Action {
        instance_name: el.attr("instanceName").map(str::to_owned),
        name: el.attr("name").map(str::to_owned),
        params: parse_params(el, class)?,
        states,
        class_name,
    })
}

fn parse_params(el: &Element, class: Option<&lynx_schema::ClassSchema>) -> Result<Vec<Param>> {
    let mut out = Vec::new();
    for p in el.children_named("parameter") {
        if !p.children.is_empty() {
            return Err(Error::invalid("lnx: <parameter> with children"));
        }
        let name = required(p, "name")?.to_owned();
        let raw = required(p, "value")?.to_owned();
        let index = match p.attr("index") {
            Some(s) => s.trim().parse().map_err(|_| Error::invalid(format!("lnx: bad index {s:?} on {name}")))?,
            None => 0,
        };
        let num_elements = match p.attr("numElements") {
            Some(s) => Some(s.trim().parse().map_err(|_| Error::invalid(format!("lnx: bad numElements {s:?}")))?),
            None => None,
        };
        let ty = param_type(class, &name).ok_or_else(|| Error::invalid(format!("lnx: untyped parameter {name}")))?;
        let (value, exact) =
            parse_value(ty, &raw).ok_or_else(|| Error::invalid(format!("lnx: {name} = {raw:?} is not {ty:?}")))?;
        out.push(Param { name, index, num_elements, raw, value, exact });
    }
    Ok(out)
}

pub fn param_type(class: Option<&lynx_schema::ClassSchema>, name: &str) -> Option<ParamType> {
    class.and_then(|c| c.param(name)).map(|p| p.ty).or_else(|| ParamType::from_name(name))
}

pub fn parse_value(ty: ParamType, raw: &str) -> Option<(Value, bool)> {
    Some(match ty {
        ParamType::Bool => (Value::Bool(!(raw == "false" || raw == "0")), true),
        ParamType::Int => {
            let (v, n) = c_number(raw, false)?;
            (Value::Int(v as i32), raw[n..].trim().is_empty())
        }
        ParamType::Float => {
            let (v, n) = c_number(raw, true)?;
            (Value::Float(v as f32), raw[n..].trim().is_empty())
        }
        ParamType::Vector2 | ParamType::Vector3 | ParamType::Vector4 => {
            let inner = raw.trim().strip_prefix('{')?.strip_suffix('}')?;
            let mut exact = true;
            let mut comps = Vec::new();
            for c in inner.split(',') {
                let (v, n) = c_number(c, true)?;
                exact &= c[n..].trim().is_empty();
                comps.push(v as f32);
            }
            let (lo, hi) = match ty {
                ParamType::Vector2 => (2, 2),
                ParamType::Vector3 => (3, 3),
                _ => (3, 4),
            };
            if comps.len() < lo || comps.len() > hi {
                return None;
            }
            (Value::Vector(comps), exact)
        }
        ParamType::Reference => (Value::Reference(raw.to_owned()), true),
        ParamType::String64 => (Value::String(raw.to_owned()), raw.len() < 64),
        ParamType::Matrix33 | ParamType::Matrix44 => return None,
    })
}

fn c_number(s: &str, float: bool) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let s = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - s
    };
    let mut nd = digits(&mut i);
    if float {
        if i < b.len() && b[i] == b'.' {
            i += 1;
            nd += digits(&mut i);
        }
        if nd > 0 && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if digits(&mut j) > 0 {
                i = j;
            }
        }
    }
    if nd == 0 {
        return None;
    }
    Some((s[start..i].parse().ok()?, i))
}

#[derive(Clone, Debug)]
pub struct Behaviour {
    pub name: String,
    pub variables: Vec<Variable>,
    pub triggers: Vec<Trigger>,
    pub blend_states: Vec<BlendState>,
}

#[derive(Clone, Debug)]
pub struct Variable {
    pub name: String,
    pub ty: String,
    pub source: Option<String>,
    pub default: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub kind: String,
    pub conditions: Vec<Expression>,
}

#[derive(Clone, Debug)]
pub struct BlendState {
    pub axes: Vec<BlendAxis>,
}

#[derive(Clone, Debug)]
pub struct BlendAxis {
    pub weight: Expression,
    pub from: [f32; 2],
    pub to: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Expression {
    pub text: String,
    pub tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ident(String),
    Number(f32),
    Op(&'static str),
}

impl Expression {
    pub fn parse(text: &str) -> Result<Expression> {
        const OPS: [&str; 15] = ["&&", "||", "<=", ">=", "==", "!=", "(", ")", "!", "+", "-", "*", "/", "<", ">"];
        let mut tokens = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c.is_ascii_alphabetic() || c == b'_' {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                tokens.push(Token::Ident(text[s..i].to_owned()));
            } else if c.is_ascii_digit() || c == b'.' {
                let s = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                let n = text[s..i].parse().map_err(|_| Error::invalid(format!("pbs: bad number in {text:?}")))?;
                tokens.push(Token::Number(n));
            } else if let Some(op) = OPS.iter().find(|op| text[i..].starts_with(**op)) {
                tokens.push(Token::Op(op));
                i += op.len();
            } else {
                return Err(Error::invalid(format!("pbs: unexpected {:?} in {text:?}", c as char)));
            }
        }
        Ok(Expression { text: text.to_owned(), tokens })
    }

    pub fn idents(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().filter_map(|t| match t {
            Token::Ident(s) => Some(s.as_str()),
            _ => None,
        })
    }
}

fn pair(el: &Element, attr: &str) -> Result<[f32; 2]> {
    let s = required(el, attr)?;
    let v: Vec<f32> = s
        .split(',')
        .map(|c| c.trim().parse::<f32>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::invalid(format!("pbs: bad {attr}={s:?}")))?;
    match v[..] {
        [a, b] => Ok([a, b]),
        _ => Err(Error::invalid(format!("pbs: {attr}={s:?} is not a pair"))),
    }
}

impl Behaviour {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(data).map_err(|_| Error::invalid("pbs: not UTF-8"))?;
        let root = xml::parse(text)?;
        if root.name != "particleBehaviour" {
            return Err(Error::invalid(format!("pbs: root element {}", root.name)));
        }
        only_children(&root, &["variable", "trigger", "blendstate"])?;
        let mut b = Behaviour {
            name: required(&root, "name")?.to_owned(),
            variables: vec![],
            triggers: vec![],
            blend_states: vec![],
        };
        for el in &root.children {
            match el.name.as_str() {
                "variable" => b.variables.push(Variable {
                    name: required(el, "name")?.to_owned(),
                    ty: required(el, "type")?.to_owned(),
                    source: el.attr("source").map(str::to_owned),
                    default: el.attr("default").map(str::to_owned),
                }),
                "trigger" => {
                    only_children(el, &["condition"])?;
                    let conditions = el
                        .children_named("condition")
                        .map(|c| Expression::parse(required(c, "expression")?))
                        .collect::<Result<_>>()?;
                    b.triggers.push(Trigger { kind: required(el, "type")?.to_owned(), conditions });
                }
                _ => {
                    let mut axes = Vec::new();
                    for k in 0..2 {
                        let Some(w) = el.attr(&format!("weight{k}")) else { break };
                        axes.push(BlendAxis {
                            weight: Expression::parse(w)?,
                            from: pair(el, &format!("from{k}"))?,
                            to: pair(el, &format!("to{k}"))?,
                        });
                    }
                    if axes.is_empty() {
                        return Err(Error::invalid("pbs: <blendstate> without weight0"));
                    }
                    b.blend_states.push(BlendState { axes });
                }
            }
        }
        Ok(b)
    }

    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_the_c_conversions() {
        assert_eq!(parse_value(ParamType::Bool, "0"), Some((Value::Bool(false), true)));
        assert_eq!(parse_value(ParamType::Bool, "false"), Some((Value::Bool(false), true)));
        assert_eq!(parse_value(ParamType::Bool, "2"), Some((Value::Bool(true), true)));
        assert_eq!(parse_value(ParamType::Int, " -150 "), Some((Value::Int(-150), true)));
        assert_eq!(parse_value(ParamType::Int, "1.5"), Some((Value::Int(1), false)));
        assert_eq!(parse_value(ParamType::Float, "1e-3"), Some((Value::Float(1e-3), true)));
        assert_eq!(parse_value(ParamType::Float, "x"), None);
        let v = parse_value(ParamType::Vector4, "{ 1.0, 0.75, 0.25, 0.2=5 }").unwrap();
        assert_eq!(v, (Value::Vector(vec![1.0, 0.75, 0.25, 0.2]), false));
        assert_eq!(parse_value(ParamType::Vector4, "{0, -6, 0 }"), Some((Value::Vector(vec![0.0, -6.0, 0.0]), true)));
        assert_eq!(parse_value(ParamType::Vector3, "{0, 1}"), None);
    }

    #[test]
    fn parses_a_minimal_system_and_behaviour() {
        let lnx = br#"<particleSystem><particleEffect name="e"><particleGroup name="g">
            <parameter index="0" name="iNumParticlesMax" value="10" />
            <particleAction className="ParticleActionEmitBox"><parameter name="iEmitRate" value="5" />
            <state id="0001"><parameter index="0" name="iEmitRate" value="7" /></state></particleAction>
            </particleGroup></particleEffect></particleSystem>"#;
        let s = ParticleSystem::parse(lnx).unwrap();
        let a = s.effect("e").unwrap().group("g").unwrap().action("ParticleActionEmitBox").unwrap();
        assert_eq!(a.param("iEmitRate", 0).unwrap().value, Value::Int(5));
        assert_eq!(a.state("0001").unwrap().params[0].value, Value::Int(7));
        let pbs = br#"<particleBehaviour name="b"><variable name="A" type="BOOL" source="gamestate:BLN_InAir"/>
            <trigger type="TURN_ON"><condition expression="A &amp;&amp; !A"/></trigger>
            <blendstate weight0="A * 2" from0="0,1" to0="1,2"/></particleBehaviour>"#;
        let b = Behaviour::parse(pbs).unwrap();
        assert_eq!(b.triggers[0].conditions[0].tokens.len(), 4);
        assert_eq!(b.blend_states[0].axes[0].to, [1.0, 2.0]);
    }
}
