package com.example;

public interface BaseRepository<T> {
    T findById(Long id);
    void save(T entity);
}
