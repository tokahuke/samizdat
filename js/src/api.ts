export async function call(method: string, route: string, payload?: Object) {
  return await fetch(`http://localhost:4510${route}`, {
    method,
    body: payload ? JSON.stringify(payload) : undefined,
  });
}

export async function callRaw(method: string, route: string, body: BodyInit) {
  return await fetch(`http://localhost:4510${route}`, {
    method,
    body,
  });
}
