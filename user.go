package main

import "log"

type user struct {
	name     string
	phone    string
	password string
	level    uint8
}

func newUser(p, pwd string) *user {
	return &user{
		name:     "",
		phone:    p,
		password: pwd,
		level:    0,
	}
}

func (u *user) update(field string, val interface{}) *user {
	switch field {
	case "name":
		u.name = val.(string)
	case "phone":
		u.phone = val.(string)
	case "password":
		u.password = val.(string)
	case "level":
		u.level = val.(uint8)
	default:
		log.Panic("unrecognized user property")
	}
	return u
}
