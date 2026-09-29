// Package operationsschema owns the embedded Operations database schema.
package operationsschema

import "embed"

// Files contains the current installation baseline. When a dated change set is
// added, include its `*.mariadb.sql` files here as an additional embed pattern.
//
//go:embed init.mariadb.sql 202609060001/*.mariadb.sql 202609060002/*.mariadb.sql 202609060003/*.mariadb.sql 202609060004/*.mariadb.sql 202609060005/*.mariadb.sql 202609060006/*.mariadb.sql 202609060007/*.mariadb.sql 202609060008/*.mariadb.sql 202609062050/*.mariadb.sql 202609062300/*.mariadb.sql 202609070100/*.mariadb.sql 202609070200/*.mariadb.sql 202609070300/*.mariadb.sql 202609081100/*.mariadb.sql 202609110900/*.mariadb.sql 202609131200/*.mariadb.sql
var Files embed.FS
