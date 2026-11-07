use std::collections::BTreeMap;

use crate::manifest::Manifest;
use crate::version::{Version, VersionReq};

pub const API_LEVEL: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DependencyError {
    #[error("two packages are named {0}")]
    DuplicateName(String),
    #[error("{package} needs API level {needs}; this game offers {API_LEVEL}")]
    ApiLevel { package: String, needs: u32 },
    #[error("{package} requires {name} {req}, which is not installed")]
    Missing { package: String, name: String, req: VersionReq },
    #[error("{package} requires {name} {req}; the installed version is {found}")]
    Version { package: String, name: String, req: VersionReq, found: Version },
    #[error("{package} requires {name} {req}; {providers} only provide the name, so a range cannot be checked")]
    ProvidedOnly { package: String, name: String, req: VersionReq, providers: String },
    #[error("{name} is provided by more than one package ({providers})")]
    AmbiguousProvide { name: String, providers: String },
    #[error("the packages require each other in a cycle: {0}")]
    Cycle(String),
}

fn provider<'a>(
    by_name: &BTreeMap<&'a str, usize>,
    provides: &BTreeMap<&'a str, Vec<usize>>,
    name: &str,
) -> Option<Result<usize, Vec<usize>>> {
    if let Some(&i) = by_name.get(name) {
        return Some(Ok(i));
    }
    match provides.get(name).map(Vec::as_slice) {
        Some([one]) => Some(Ok(*one)),
        Some(many) if !many.is_empty() => Some(Err(many.to_vec())),
        _ => None,
    }
}

pub fn start_order(packages: &[Manifest]) -> Result<Vec<usize>, Vec<DependencyError>> {
    let mut errors = Vec::new();
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
    let mut provides: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, m) in packages.iter().enumerate() {
        let name = m.resource.name.as_str();
        if by_name.insert(name, i).is_some() {
            errors.push(DependencyError::DuplicateName(name.to_string()));
        }
        if m.resource.api > API_LEVEL {
            errors.push(DependencyError::ApiLevel { package: name.to_string(), needs: m.resource.api });
        }
        for p in &m.dependencies.provide {
            provides.entry(p.as_str()).or_default().push(i);
        }
    }
    let names = |is: &[usize]| is.iter().map(|&i| packages[i].resource.name.as_str()).collect::<Vec<_>>().join(", ");
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); packages.len()];
    for (i, m) in packages.iter().enumerate() {
        let package = m.resource.name.clone();
        for d in &m.dependencies.require {
            let (name, req) = (d.name.clone(), d.req.clone());
            match provider(&by_name, &provides, &d.name) {
                None => errors.push(DependencyError::Missing { package: package.clone(), name, req }),
                Some(Err(many)) => errors.push(DependencyError::AmbiguousProvide { name, providers: names(&many) }),
                Some(Ok(j)) if packages[j].resource.name != d.name && !req.comparators.is_empty() => {
                    errors.push(DependencyError::ProvidedOnly {
                        package: package.clone(),
                        name,
                        req,
                        providers: names(&[j]),
                    })
                }
                Some(Ok(j)) if !req.matches(packages[j].resource.version) => {
                    let found = packages[j].resource.version;
                    errors.push(DependencyError::Version { package: package.clone(), name, req, found });
                }
                Some(Ok(j)) => edges[i].push(j),
            }
        }
    }
    let mut state = vec![0u8; packages.len()];
    let mut order = Vec::with_capacity(packages.len());
    for start in 0..packages.len() {
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(node, next)) = stack.last() {
            if next == 0 && state[node] != 0 {
                stack.pop();
                continue;
            }
            state[node] = 1;
            let Some(&dep) = edges[node].get(next) else {
                state[node] = 2;
                order.push(node);
                stack.pop();
                continue;
            };
            let top = stack.len() - 1;
            stack[top].1 += 1;
            match state[dep] {
                0 => stack.push((dep, 0)),
                1 => {
                    let from = stack.iter().position(|&(n, _)| n == dep).unwrap_or(0);
                    let cycle = stack[from..].iter().map(|&(n, _)| n).chain([dep]);
                    let shown: Vec<&str> = cycle.map(|n| packages[n].resource.name.as_str()).collect();
                    errors.push(DependencyError::Cycle(shown.join(" -> ")));
                }
                _ => {}
            }
        }
    }
    if errors.is_empty() { Ok(order) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn pkg(name: &str, version: &str, extra: &str) -> Manifest {
        let text = format!("[resource]\nname = \"{name}\"\nversion = \"{version}\"\napi = 1\n{extra}");
        Manifest::parse(&text, Path::new("resource.toml")).unwrap()
    }

    fn order(ps: &[Manifest]) -> Vec<&str> {
        start_order(ps).unwrap().into_iter().map(|i| ps[i].resource.name.as_str()).collect()
    }

    #[test]
    fn requirements_order_the_start() {
        let ps = [
            pkg("race", "1.0.0", "[dependencies]\nrequire = [\"mapmanager >= 1.0, < 2\", \"ui\"]\n"),
            pkg("ui", "0.3.0", ""),
            pkg("mapmanager", "1.4.2", "[dependencies]\nrequire = [\"ui >= 0.3\"]\n"),
        ];
        assert_eq!(order(&ps), ["ui", "mapmanager", "race"]);
    }

    #[test]
    fn ranges_are_enforced() {
        let ps = [
            pkg("race", "1.0.0", "[dependencies]\nrequire = [\"mapmanager >= 1.0, < 2\"]\n"),
            pkg("mapmanager", "2.0.0", ""),
        ];
        let e = start_order(&ps).unwrap_err();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].to_string(), "race requires mapmanager >= 1.0.0, < 2.0.0; the installed version is 2.0.0");
        let e = start_order(&ps[..1]).unwrap_err();
        assert!(matches!(&e[0], DependencyError::Missing { name, .. } if name == "mapmanager"));
    }

    #[test]
    fn provided_names() {
        let lib = pkg("fastmaps", "3.0.0", "[dependencies]\nprovide = [\"mapmanager\"]\n");
        assert_eq!(
            order(&[pkg("race", "1.0.0", "[dependencies]\nrequire = [\"mapmanager\"]\n"), lib.clone()]),
            ["fastmaps", "race"]
        );
        let ranged = [pkg("race", "1.0.0", "[dependencies]\nrequire = [\"mapmanager >= 1\"]\n"), lib.clone()];
        assert!(matches!(&start_order(&ranged).unwrap_err()[0], DependencyError::ProvidedOnly { .. }));
        let two = [
            pkg("race", "1.0.0", "[dependencies]\nrequire = [\"mapmanager\"]\n"),
            lib,
            pkg("othermaps", "1.0.0", "[dependencies]\nprovide = [\"mapmanager\"]\n"),
        ];
        assert!(matches!(&start_order(&two).unwrap_err()[0], DependencyError::AmbiguousProvide { .. }));
    }

    #[test]
    fn cycles_duplicates_and_api_levels() {
        let ps = [
            pkg("a", "1.0.0", "[dependencies]\nrequire = [\"b\"]\n"),
            pkg("b", "1.0.0", "[dependencies]\nrequire = [\"c\"]\n"),
            pkg("c", "1.0.0", "[dependencies]\nrequire = [\"a\"]\n"),
        ];
        let e = start_order(&ps).unwrap_err();
        assert_eq!(e, [DependencyError::Cycle("a -> b -> c -> a".into())]);
        let e = start_order(&[pkg("a", "1.0.0", ""), pkg("a", "2.0.0", "")]).unwrap_err();
        assert_eq!(e, [DependencyError::DuplicateName("a".into())]);
        let future =
            Manifest::parse("[resource]\nname = \"f\"\nversion = \"1.0.0\"\napi = 9\n", Path::new("r")).unwrap();
        assert_eq!(start_order(&[future]).unwrap_err(), [DependencyError::ApiLevel { package: "f".into(), needs: 9 }]);
        assert_eq!(start_order(&[]).unwrap(), Vec::<usize>::new());
    }
}
