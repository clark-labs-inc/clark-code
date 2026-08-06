type Session = { userId: string; tenantId: string };
type User = { id: string; tenantId: string; email: string };
declare function findUser(id: string): Promise<User | null>;

export async function getUser(_session: Session, requestedUserId: string) {
  // Vulnerable: authentication exists, but ownership and tenant checks do not.
  return findUser(requestedUserId);
}
