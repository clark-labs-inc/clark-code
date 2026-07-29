declare function query(sql: string, values: unknown[]): Promise<unknown[]>;

export function searchPublicCatalog(term: string) {
  return query("SELECT * FROM catalog WHERE name ILIKE $1", [`%${term}%`]);
}
