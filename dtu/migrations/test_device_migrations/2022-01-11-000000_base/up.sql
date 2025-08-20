INSERT INTO system_services (id, name, iface, can_get_binder)
VALUES (0, 'test_can', 'test.can.IFace', 1);

INSERT INTO system_services (id, name, iface, can_get_binder)
VALUES (1, 'test_cant', 'test.cant.IFace', 0);

INSERT INTO system_services (id, name, iface, can_get_binder)
VALUES (2, 'test_no_iface', NULL, 1);

INSERT INTO apks (id, app_name, name, is_debuggable)
VALUES (0, "just.an.app", "JustAnApp.apk", false);

INSERT INTO apks (id, app_name, name, is_debuggable)
VALUES (1, "is.debuggable", "Debuggable.apk", true);

-- For authority testing
INSERT INTO providers(id, name, authorities, grant_uri_permissions, exported, enabled, apk_id)
VALUES(1, "exact", "exact.authority", true, true, true, 0);

INSERT INTO providers(id, name, authorities, grant_uri_permissions, exported, enabled, apk_id)
VALUES(2, "left", "left.authority:not.left.authority", true, true, true, 0);

INSERT INTO providers(id, name, authorities, grant_uri_permissions, exported, enabled, apk_id)
VALUES(3, "middle", "left.not.middle.authority:middle.authority:right.not.middle.authority", true, true, true, 0);

INSERT INTO providers(id, name, authorities, grant_uri_permissions, exported, enabled, apk_id)
VALUES(4, "right", "not.right.authority:right.authority", true, true, true, 0);