declare extern type Element with
end

declare extern type Timer with
end

declare extern type LatentHandle with
end

type NetValue = nil | boolean | number | string | Element | { [any]: any }
type BindableKey = "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f10" | "f11" | "f12" | "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "num_0" | "num_1" | "num_2" | "num_3" | "num_4" | "num_5" | "num_6" | "num_7" | "num_8" | "num_9" | "num_add" | "num_sub" | "num_mul" | "num_div" | "num_dec" | "b" | "m" | "o" | "p" | "u" | "," | "." | "/" | ";" | "'" | "[" | "]" | "-" | "=" | "`" | "\\" | "insert" | "pause" | "scroll"
type LatentStatus = { total: number, sent: number, percent: number }
type Vec3 = { x: number, y: number, z: number }
type RiderState = { name: string, position: Vec3, velocity: Vec3?, speed: number?, air: boolean?, grind: boolean?, trick: string?, score: number?, health: number?, finished: boolean?, isLocal: boolean }
type Instance = { mode: string, drop: string, state: "lobby" | "countdown" | "running" | "results", clock: number }
type Standing = { rider: Element?, name: string, place: number, value: number, finished: boolean }
type ContentInfo = { id: string, kind: string, source: string, layer: string }
type InstanceInfo = { id: number, mode: string, drop: string, state: "lobby" | "loading" | "countdown" | "running" | "results", round: number, clock: number }
type ServerStanding = { rider: Element?, name: string, place: number, state: "racing" | "finished" | "failed" | "left" | "invalidated", clock: number, points: number, split: number? }
type ServerRiderState = { name: string, number: number, progress: "joined" | "dressed" | "loaded" | "ready" | "away", status: "joining" | "loading" | "intro" | "riding" | "away" | "spectating" | "finished", ping: number, position: Vec3?, velocity: Vec3?, speed: number?, split: number?, clock: number?, points: number?, finished: boolean }
type MarkerOptions = { heading: number?, size: number?, color: number?, visible: boolean? }
type ServerMarkerOptions = { heading: number?, size: number?, color: number?, visible: boolean?, instance: Element? }
type TextOptions = { size: number?, align: ("left" | "center" | "right")?, shadow: boolean? }
type OnClientResourceStartHandler = () -> ()
type OnClientResourceStopHandler = () -> ()
type OnClientElementDataChangeHandler = (key: string, old: NetValue, new: NetValue) -> ()
type OnClientRenderHandler = () -> ()
type OnClientKeyHandler = (key: BindableKey, down: boolean) -> ()
type OnClientRiderLandHandler = (airTime: number, bail: boolean) -> ()
type OnClientTrickHandler = (name: string, points: number) -> ()
type OnClientMarkerHitHandler = (rider: Element, forward: boolean) -> ()

declare root: Element
declare resourceRoot: Element
declare source: Element
declare this: Element
declare eventName: string
declare localPlayer: Element?
declare exports: ((name: string, fn: (...any) -> ...any) -> ()) & { [string]: { [string]: (self: any, ...any) -> ...any } }

declare function getResourceName(): string
declare function setTimer(callback: (...any) -> ...any, interval: number, times: number, ...: any): Timer
declare function killTimer(timer: Timer): boolean
declare function isTimer(value: any): boolean
declare function addCommandHandler(name: string, handler: (...any) -> ...any): boolean
declare function removeCommandHandler(name: string, handler: ((...any) -> ...any)?): boolean
declare function get(name: string): NetValue
declare function createElement(type: string, id: string?): Element
declare function destroyElement(element: Element): boolean
declare function isElement(value: any): boolean
declare function getElementType(element: Element): string
declare function getElementID(element: Element): string
declare function setElementID(element: Element, id: string): boolean
declare function getElementByID(id: string, index: number?): Element?
declare function getElementParent(element: Element): Element?
declare function setElementParent(element: Element, parent: Element): boolean
declare function getElementChildren(element: Element, type: string?): { Element }
declare function getElementsByType(type: string, startAt: Element?): { Element }
declare function getRootElement(): Element
declare function getResourceRootElement(): Element
declare function setElementData(element: Element, key: string, value: NetValue, sync: ("local" | "subscribers" | "broadcast")?): boolean
declare function getElementData(element: Element, key: string): NetValue
declare function hasElementData(element: Element, key: string): boolean
declare function removeElementData(element: Element, key: string): boolean
declare function addEvent(name: string, remote: boolean?): boolean
declare addEventHandler: ((name: "onClientResourceStart", element: Element, handler: OnClientResourceStartHandler, propagate: boolean?) -> boolean) & ((name: "onClientResourceStop", element: Element, handler: OnClientResourceStopHandler, propagate: boolean?) -> boolean) & ((name: "onClientElementDataChange", element: Element, handler: OnClientElementDataChangeHandler, propagate: boolean?) -> boolean) & ((name: "onClientRender", element: Element, handler: OnClientRenderHandler, propagate: boolean?) -> boolean) & ((name: "onClientKey", element: Element, handler: OnClientKeyHandler, propagate: boolean?) -> boolean) & ((name: "onClientRiderLand", element: Element, handler: OnClientRiderLandHandler, propagate: boolean?) -> boolean) & ((name: "onClientTrick", element: Element, handler: OnClientTrickHandler, propagate: boolean?) -> boolean) & ((name: "onClientMarkerHit", element: Element, handler: OnClientMarkerHitHandler, propagate: boolean?) -> boolean) & ((name: string, element: Element, handler: (...any) -> ...any, propagate: boolean?) -> boolean)
declare function removeEventHandler(name: string, element: Element, handler: (...any) -> ...any): boolean
declare function triggerEvent(name: string, source: Element, ...: NetValue): boolean
declare function cancelEvent(): boolean
declare function wasEventCancelled(): boolean
declare function triggerServerEvent(name: string, source: Element, ...: NetValue): boolean
declare function triggerLatentServerEvent(name: string, bandwidth: number, source: Element, ...: NetValue): LatentHandle
declare function getLatentEventStatus(handle: LatentHandle): LatentStatus?
declare function cancelLatentEvent(handle: LatentHandle): boolean
declare function getLatentEventHandles(): { LatentHandle }
declare function getLocalRider(): Element?
declare function getRiders(): { Element }
declare function getRiderState(rider: Element): RiderState?
declare function getInstance(): Instance?
declare function getStandings(): { Standing }
declare function getContent(id: string): ContentInfo?
declare function listContent(kind: "texture" | "model" | "object" | "material" | "character" | "gear" | "board" | "anim" | "track" | "drop" | "event" | "lighting" | "sky" | "postfx" | "particles" | "sound" | "music" | "movie" | "ui" | "font" | "string" | "tuning" | "scoring" | "script" | "marker" | "map" | "gamemode" | "trick"): { string }
declare function preloadContent(id: string): boolean
declare function getGroundHeight(x: number, z: number): number?
declare function musicPlayer(action: string, id: string?): boolean
declare function musicVolume(level: number): boolean
declare function createMarker(id: string, position: Vec3, options: MarkerOptions?): Element
declare function setMarkerPosition(marker: Element, position: Vec3): boolean
declare function getMarkerPosition(marker: Element): Vec3
declare function setMarkerColor(marker: Element, color: number): boolean
declare function setMarkerVisible(marker: Element, visible: boolean): boolean
declare function bindKey(key: BindableKey, state: "down" | "up" | "both", handler: (...any) -> ...any): boolean
declare function unbindKey(key: BindableKey, state: ("down" | "up" | "both")?, handler: ((...any) -> ...any)?): boolean
declare function setCameraTarget(rider: Element?): boolean
declare function fadeCamera(fadeIn: boolean, seconds: number?, color: number?): boolean
declare function playSound(id: string, position: Vec3?): boolean
declare function playMusic(id: string): boolean
declare function showHudMessage(title: string, text: string, seconds: number?): boolean
declare function tocolor(r: number, g: number, b: number, a: number?): number

declare draw: {
    text: (text: string, x: number, y: number, color: number?, options: TextOptions?) -> boolean,
    rect: (x: number, y: number, w: number, h: number, color: number) -> boolean,
    image: (x: number, y: number, w: number, h: number, texture: string, color: number?) -> boolean,
}

declare json: {
    encode: (value: NetValue) -> string,
    decode: (text: string) -> NetValue,
}

declare storage: {
    get: (key: string) -> NetValue,
    set: (key: string, value: NetValue) -> (),
}
