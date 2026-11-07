use ssx_net_proto::scripts::{ScriptBody, ScriptItem, WireMarker};

use super::outbox::{Body, MarkerSpec};
use crate::value::ElementId;

const _: () = assert!(super::limits::CLIENT_LATENT_TOTAL == ssx_net_proto::scripts::LATENT_UP);

pub fn to_item(package: Option<&str>, body: &Body) -> ScriptItem {
    let e = |id: &ElementId| id.0;
    let body = match body {
        Body::Event { name, source, args } => {
            ScriptBody::Event { name: name.clone(), source: e(source), args: args.clone() }
        }
        Body::LatentStart { transfer, name, source, total } => {
            ScriptBody::LatentStart { transfer: *transfer, name: name.clone(), source: e(source), total: *total }
        }
        Body::LatentChunk { transfer, offset, bytes } => {
            ScriptBody::LatentChunk { transfer: *transfer, offset: *offset, bytes: bytes.clone() }
        }
        Body::LatentCancel { transfer } => ScriptBody::LatentCancel { transfer: *transfer },
        Body::DataWrite { element, key, value } => {
            ScriptBody::DataWrite { element: e(element), key: key.clone(), value: value.clone() }
        }
        Body::Reset => ScriptBody::Reset,
        Body::Created { element, kind, name, parent } => {
            ScriptBody::Created { element: e(element), kind: kind.clone(), name: name.clone(), parent: e(parent) }
        }
        Body::Destroyed { element } => ScriptBody::Destroyed { element: e(element) },
        Body::Moved { element, parent } => ScriptBody::Moved { element: e(element), parent: e(parent) },
        Body::Renamed { element, name } => ScriptBody::Renamed { element: e(element), name: name.clone() },
        Body::Data { element, key, value } => {
            ScriptBody::Data { element: e(element), key: key.clone(), value: value.clone() }
        }
        Body::Hello { player } => ScriptBody::Hello { player: e(player) },
        Body::Rider { element, player, number } => {
            ScriptBody::Rider { element: e(element), player: e(player), number: *number }
        }
        Body::Marker { element, spec: s } => ScriptBody::Marker {
            element: e(element),
            marker: WireMarker {
                id: s.id.clone(),
                position: s.position,
                heading: s.heading,
                size: s.size,
                colour: s.colour,
                visible: s.visible,
                checkpoint: s.checkpoint,
                instance: s.instance.as_ref().map(e),
            },
        },
        Body::Notice { title, text, seconds } => {
            ScriptBody::Notice { title: title.clone(), text: text.clone(), seconds: *seconds }
        }
        Body::Sound { id, position } => ScriptBody::Sound { id: id.clone(), position: *position },
        Body::Started => ScriptBody::Started,
    };
    ScriptItem { package: package.map(str::to_string), body }
}

pub fn from_item(item: ScriptItem) -> (Option<String>, Body) {
    let e = ElementId;
    let body = match item.body {
        ScriptBody::Event { name, source, args } => Body::Event { name, source: e(source), args },
        ScriptBody::LatentStart { transfer, name, source, total } => {
            Body::LatentStart { transfer, name, source: e(source), total }
        }
        ScriptBody::LatentChunk { transfer, offset, bytes } => Body::LatentChunk { transfer, offset, bytes },
        ScriptBody::LatentCancel { transfer } => Body::LatentCancel { transfer },
        ScriptBody::DataWrite { element, key, value } => Body::DataWrite { element: e(element), key, value },
        ScriptBody::Reset => Body::Reset,
        ScriptBody::Created { element, kind, name, parent } => {
            Body::Created { element: e(element), kind, name, parent: e(parent) }
        }
        ScriptBody::Destroyed { element } => Body::Destroyed { element: e(element) },
        ScriptBody::Moved { element, parent } => Body::Moved { element: e(element), parent: e(parent) },
        ScriptBody::Renamed { element, name } => Body::Renamed { element: e(element), name },
        ScriptBody::Data { element, key, value } => Body::Data { element: e(element), key, value },
        ScriptBody::Hello { player } => Body::Hello { player: e(player) },
        ScriptBody::Rider { element, player, number } => Body::Rider { element: e(element), player: e(player), number },
        ScriptBody::Marker { element, marker: m } => Body::Marker {
            element: e(element),
            spec: MarkerSpec {
                id: m.id,
                position: m.position,
                heading: m.heading,
                size: m.size,
                colour: m.colour,
                visible: m.visible,
                checkpoint: m.checkpoint,
                instance: m.instance.map(ElementId),
            },
        },
        ScriptBody::Notice { title, text, seconds } => Body::Notice { title, text, seconds },
        ScriptBody::Sound { id, position } => Body::Sound { id, position },
        ScriptBody::Started => Body::Started,
    };
    (item.package, body)
}
