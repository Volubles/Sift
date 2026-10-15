CREATE INDEX samples_format_order ON samples(format,name_key,id);
CREATE INDEX samples_length_order ON samples(duration_seconds,name_key,id);
CREATE INDEX samples_rate_order ON samples(sample_rate,name_key,id);
CREATE INDEX samples_channels_order ON samples(channels,name_key,id);
PRAGMA user_version=4;
