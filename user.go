package main

import "log"

type User struct {
	name     string
	phone    string
	password string
	level    uint8
}

func newUser(p, pwd string) *User {
	return &User{
		name:     "",
		phone:    p,
		password: pwd,
		level:    0,
	}
}

func (u *User) update(field string, val interface{}) *User {
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
