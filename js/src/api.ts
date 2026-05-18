// All requests use relative URLs: the page is served by the node, so its
// own origin is correct regardless of which port the node is listening on.

export async function call(method: string, route: string, payload?: object) {
  return await fetch(route, {
    method,
    headers: {
      "Content-Type": "application/json",
    },
    body: payload !== undefined ? JSON.stringify(payload) : undefined,
  });
}

export async function callRaw(method: string, route: string, body: BodyInit) {
  return await fetch(route, {
    method,
    body,
  });
}
