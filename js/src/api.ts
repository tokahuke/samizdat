export async function call(method: string, route: string, payload?: Object) {
  const response = await fetch(`http://localhost:4510${route}`, {
    method,
    body: payload ? JSON.stringify(payload) : undefined,
  });

  return new Response(response.status, await response.text());
}

export class Response {
  __text: string;
  __status: number;

  constructor(status: number, text: string) {
    this.__text = text;
    this.__status = status;
  }

  text(): string {
    return this.__text;
  }

  json(): any {
    return JSON.parse(this.__text);
  }
}
