CREATE TABLE comparison_jobs(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,state TEXT NOT NULL,payload TEXT NOT NULL,UNIQUE(workspace,repository,request));
CREATE UNIQUE INDEX comparison_active ON comparison_jobs(workspace,repository) WHERE state='running';
CREATE TABLE research_packages(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,payload TEXT NOT NULL);
CREATE TABLE comparison_records(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,package TEXT NOT NULL REFERENCES research_packages(id),payload TEXT NOT NULL);
CREATE TABLE comparison_entries(package TEXT NOT NULL REFERENCES research_packages(id),section TEXT NOT NULL,ordinal INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(package,section,ordinal));
CREATE TABLE comparison_dependencies(package TEXT NOT NULL REFERENCES research_packages(id),dataset TEXT NOT NULL REFERENCES app_records(id),PRIMARY KEY(package,dataset));
PRAGMA user_version=9;
