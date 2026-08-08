type Session = { tenantId: string };
declare function query(sql: string, values: unknown[]): Promise<unknown[]>;

export async function report(_session: Session, reportId: string) {
  // Vulnerable: reportId is parameterized, but tenant ownership is not checked.
  return query("SELECT * FROM reports WHERE id = $1", [reportId]);
}
