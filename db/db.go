// For caching
package db

import (
	"database/sql"
	"path"

	_ "github.com/mattn/go-sqlite3"
)

type TDB struct {
	*sql.DB
}

func InitDB(appDir string) (*TDB, error) {

	dbpath := path.Join(appDir, "terminews.db")
	db, err := sql.Open("sqlite3", dbpath)
	if err != nil || db == nil {
		return nil, err
	}

	tdb := &TDB{db}
	if err = tdb.CreateTables(); err != nil {
		return nil, err
	}

	return tdb, nil
}

func (tdb *TDB) CreateTables() error {
	ssql := []string{
		GetSiteSql(),
		GetEventSql(),
	}
	for _, s := range ssql {
		_, err := tdb.Exec(s)
		if err != nil {
			return err
		}
	}

	return nil
}

func (tdb *TDB) DropTables() error {
	ssql := []string{
		"DROP TABLE site;",
		"DROP TABLE event;",
	}
	for _, s := range ssql {
		_, err := tdb.Exec(s)
		if err != nil {
			return err
		}
	}

	return nil
}
