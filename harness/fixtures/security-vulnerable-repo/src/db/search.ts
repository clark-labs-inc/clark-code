declare function query(sql: string): Promise<unknown[]>;

export async function searchInvoices(term: string) {
  // Vulnerable: tenant-controlled text is concatenated into executable SQL.
  return query(`SELECT * FROM invoices WHERE memo LIKE '%${term}%'`);
}
