import { AccessRight } from "./index";

async function isAuthenticated(accessRights: Array<AccessRight>) {
  const response = await fetch("/_auth/_current");

  if (response.status == 200) {
    const grantedRights: Array<AccessRight> = (await response.json())["Ok"];
    for (const right of accessRights) {
      if (!grantedRights.includes(right)) {
        return false;
      }
    }

    return true;
  }

  return false;
}

async function doAuthenticationFlow(accessRights: Array<AccessRight>) {
  interface AuthenticationDetail {
    status: "success" | "fail" | "canceled";
    statusCode: number,
  }

  const screen = window.screen;
  const { width, height } = {
    width: screen.width / 4.0,
    height: (screen.height * 2.0) / 3.0,
  };
  const query = accessRights.map(right => `right=${right}`).join("&");
  const authWindow = window.open(
    `/_register?${query}`,
    "RegisterApp",
    `
      left=${(screen.width - width) / 2.0},
      top=${(screen.height - height) / 4.0},
      width=${width},
      height=${height}
    `
  );

  if (!authWindow) {
    throw new Error("Could not open authentication window");
  }

  const event = await new Promise<CustomEvent<AuthenticationDetail>>(
    (resolver) => {
      authWindow.addEventListener(
        "auth",
        (e: CustomEvent<AuthenticationDetail>) => resolver(e)
      );
    }
  );

  console.log(event);

  switch (event.detail.status) {
    case "canceled":
      throw new Error("User canceled authentication flow");
    case "fail":
      throw new Error("Authentication flow failed");
    case "success":
      return;
  }
}

export async function authenticate(accessRights: Array<AccessRight>) {
  if (!(await isAuthenticated(accessRights))) {
    await doAuthenticationFlow(accessRights);
  }
}
