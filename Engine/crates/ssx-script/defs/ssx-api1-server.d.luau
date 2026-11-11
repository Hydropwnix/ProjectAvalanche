declare extern type Element with
end

declare extern type Timer with
end

declare extern type LatentHandle with
end

type NetValue = nil | boolean | number | string | Element | { [any]: any }
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
type Screen = { width: number, height: number, left: number, top: number, right: number, bottom: number }
type Cursor = { x: number, y: number }
type OnResourceStartHandler = () -> ()
type OnResourceStopHandler = () -> ()
type OnElementDataChangeHandler = (key: string, old: NetValue, new: NetValue) -> ()
type OnPlayerConnectingHandler = (name: string, keyId: string, join: number) -> ()
type OnPlayerJoinHandler = () -> ()
type OnPlayerQuitHandler = (reason: string) -> ()
type OnRiderCheckpointHandler = (checkpoint: Element?, index: number, clock: number) -> ()
type OnRiderFinishHandler = (clock: number, points: number, failed: boolean) -> ()
type OnRoundStartHandler = (round: number, drop: string) -> ()
type OnRoundEndHandler = (round: number) -> ()
type OnMarkerHitHandler = (rider: Element, forward: boolean) -> ()
type OnPlayerTriggerInvalidEventHandler = (eventName: string, isAdded: boolean, isRemote: boolean, package: string) -> ()
type OnPlayerEventFloodHandler = (dropped: number) -> ()
type OnResourcePreStartHandler = (resource: string) -> ()
type OnResourceStateChangeHandler = (resource: string, old: "loaded" | "starting" | "running" | "stopping" | "stopped", new: "loaded" | "starting" | "running" | "stopping" | "stopped") -> ()
type OnPlayerResourceStartHandler = (resource: string) -> ()
type OnShutdownHandler = (roundEndSecs: number) -> ()

declare root: Element
declare resourceRoot: Element
declare source: Element
declare this: Element
declare eventName: string
declare client: Element?
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
declare function setElementOwner(element: Element, player: Element?): boolean
declare function getElementOwner(element: Element): Element?
declare function setElementData(element: Element, key: string, value: NetValue, sync: ("local" | "subscribers" | "broadcast")?): boolean
declare function getElementData(element: Element, key: string): NetValue
declare function hasElementData(element: Element, key: string): boolean
declare function removeElementData(element: Element, key: string): boolean
declare function setElementDataClientWritable(element: Element, key: string, writable: boolean): boolean
declare function addElementDataSubscriber(element: Element, key: string, player: Element): boolean
declare function removeElementDataSubscriber(element: Element, key: string, player: Element): boolean
declare function addEvent(name: string, remote: boolean?): boolean
declare addEventHandler: ((name: "onResourceStart", element: Element, handler: OnResourceStartHandler, propagate: boolean?) -> boolean) & ((name: "onResourceStop", element: Element, handler: OnResourceStopHandler, propagate: boolean?) -> boolean) & ((name: "onElementDataChange", element: Element, handler: OnElementDataChangeHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerConnecting", element: Element, handler: OnPlayerConnectingHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerJoin", element: Element, handler: OnPlayerJoinHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerQuit", element: Element, handler: OnPlayerQuitHandler, propagate: boolean?) -> boolean) & ((name: "onRiderCheckpoint", element: Element, handler: OnRiderCheckpointHandler, propagate: boolean?) -> boolean) & ((name: "onRiderFinish", element: Element, handler: OnRiderFinishHandler, propagate: boolean?) -> boolean) & ((name: "onRoundStart", element: Element, handler: OnRoundStartHandler, propagate: boolean?) -> boolean) & ((name: "onRoundEnd", element: Element, handler: OnRoundEndHandler, propagate: boolean?) -> boolean) & ((name: "onMarkerHit", element: Element, handler: OnMarkerHitHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerTriggerInvalidEvent", element: Element, handler: OnPlayerTriggerInvalidEventHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerEventFlood", element: Element, handler: OnPlayerEventFloodHandler, propagate: boolean?) -> boolean) & ((name: "onResourcePreStart", element: Element, handler: OnResourcePreStartHandler, propagate: boolean?) -> boolean) & ((name: "onResourceStateChange", element: Element, handler: OnResourceStateChangeHandler, propagate: boolean?) -> boolean) & ((name: "onPlayerResourceStart", element: Element, handler: OnPlayerResourceStartHandler, propagate: boolean?) -> boolean) & ((name: "onShutdown", element: Element, handler: OnShutdownHandler, propagate: boolean?) -> boolean) & ((name: string, element: Element, handler: (...any) -> ...any, propagate: boolean?) -> boolean)
declare function removeEventHandler(name: string, element: Element, handler: (...any) -> ...any): boolean
declare function triggerEvent(name: string, source: Element, ...: NetValue): boolean
declare function cancelEvent(): boolean
declare function wasEventCancelled(): boolean
declare function triggerClientEvent(sendTo: Element | { Element }, name: string, source: Element, ...: NetValue): boolean
declare function triggerLatentClientEvent(sendTo: Element | { Element }, name: string, bandwidth: number, source: Element, ...: NetValue): LatentHandle
declare function getLatentEventStatus(handle: LatentHandle): LatentStatus?
declare function cancelLatentEvent(handle: LatentHandle): boolean
declare function getLatentEventHandles(): { LatentHandle }
declare function getPlayers(): { Element }
declare function getPlayerName(player: Element): string?
declare function getPlayerRider(player: Element): Element?
declare function getRiderPlayer(rider: Element): Element?
declare function getPlayerInstance(player: Element): Element?
declare function getInstances(): { Element }
declare function getInstance(instance: Element): InstanceInfo?
declare function getStandings(instance: Element): { ServerStanding }
declare function getRiders(instance: Element?): { Element }
declare function getRiderState(rider: Element): ServerRiderState?
declare function endRound(instance: Element, reason: string?): boolean
declare function createMarker(id: string, position: Vec3, options: ServerMarkerOptions?): Element
declare function createCheckpoint(id: string, position: Vec3, index: number, options: ServerMarkerOptions?): Element
declare function createTrigger(position: Vec3, radius: number, instance: Element?): Element
declare function setMarkerPosition(marker: Element, position: Vec3): boolean
declare function getMarkerPosition(marker: Element): Vec3
declare function setMarkerColor(marker: Element, color: number): boolean
declare function setMarkerVisible(marker: Element, visible: boolean): boolean
declare function playSoundFor(to: Element | { Element }, id: string, position: Vec3?): boolean
declare function showNotice(to: Element | { Element }, title: string, text: string, seconds: number?): boolean
declare function deferJoin(join: number): boolean
declare function acceptJoin(join: number): boolean
declare function refuseJoin(join: number, reason: string): boolean
declare function updateJoin(join: number, text: string): boolean
declare function kickPlayer(player: Element, reason: string?): (boolean, string?)
declare function banPlayer(player: Element, seconds: number?, reason: string?): (boolean, string?)
declare function mutePlayer(player: Element, seconds: number?, reason: string?): (boolean, string?)
declare function startResource(name: string): (boolean, string?)
declare function stopResource(name: string): (boolean, string?)
declare function restartResource(name: string): (boolean, string?)
declare function executeCommand(line: string): (boolean, string?)
declare function hasObjectPermissionTo(object: Element, right: string): boolean

declare json: {
    encode: (value: NetValue) -> string,
    decode: (text: string) -> NetValue,
}

declare storage: {
    get: (key: string) -> NetValue,
    set: (key: string, value: NetValue) -> (),
}
