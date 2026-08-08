type Response = { cookie(name: string, value: string, options?: object): void };

export function setSessionCookie(response: Response, token: string) {
  // Vulnerable: JavaScript and cleartext transport can both expose the session.
  response.cookie("session", token, { httpOnly: false, secure: false });
}
