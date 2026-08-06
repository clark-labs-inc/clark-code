type Response = { redirect(destination: string): void };

export function finishLogin(response: Response, next: string) {
  // Vulnerable: an attacker controls the post-login redirect destination.
  response.redirect(next);
}
