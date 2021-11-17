import { call } from "./api";

call("POST", "/_objects/6sH5yRMDzYKV89a3saVs3JIQrqd5wBv8Uu4QuQ/bookmark")
  .then((resp) => console.log(resp.json()));
