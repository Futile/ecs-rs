
//! Entity identifier and manager types.

#[cfg(feature="serialisation")] use cereal::{CerealData, CerealError, CerealResult};

use std::collections::hash_map::{HashMap, Values};
use std::default::Default;
use std::marker::PhantomData;
use std::ops::Deref;

use Aspect;
use BuildData;
use ComponentManager;
use EntityData;
use EntityBuilder;
use ServiceManager;
use SystemManager;

pub type Id = u64;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Entity(Id);

#[cfg(feature="serialisation")]
impl_cereal_data!(Entity(), a);

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct IndexedEntity<T: ComponentManager>(usize, Entity, PhantomData<T>);

// TODO: Cleanup
#[cfg(feature="serialisation")]
unsafe impl<T: ComponentManager> CerealData for IndexedEntity<T> {
    fn write(&self, write: &mut ::std::io::Write) -> CerealResult<()> {
        try!((self.0 as u64).write(write));
        self.1.write(write)
    }

    fn read(read: &mut ::std::io::Read) -> CerealResult<IndexedEntity<T>> {
        Ok(IndexedEntity(try!(u64::read(read)) as usize, try!(CerealData::read(read)), PhantomData))
    }
}

impl Entity
{
    pub fn nil() -> Entity
    {
        Entity(0)
    }

    /// Returns the entity's unique identifier.
    #[inline]
    pub fn id(&self) -> Id
    {
        self.0
    }
}

impl<T: ComponentManager> IndexedEntity<T>
{
    pub fn index(&self) -> usize
    {
        self.0
    }

    #[doc(hidden)]
    pub fn __clone(&self) -> IndexedEntity<T>
    {
        IndexedEntity(self.0, self.1, self.2)
    }
}

impl<T: ComponentManager> Deref for IndexedEntity<T>
{
    type Target = Entity;
    fn deref(&self) -> &Entity
    {
        &self.1
    }
}

impl Default for Entity
{
    fn default() -> Entity
    {
        Entity::nil()
    }
}

pub struct FilteredEntityIter<'a, T: ComponentManager>
{
    inner: EntityIter<'a, T>,
    aspect: Aspect<T>,
    components: &'a T,
}

// Inner Entity Iterator
pub enum EntityIter<'a, T: ComponentManager>
{
    Map(Values<'a, Entity, IndexedEntity<T>>),
}

impl<'a, T: ComponentManager> EntityIter<'a, T>
{
    pub fn filter(self, aspect: Aspect<T>, components: &'a T) -> FilteredEntityIter<'a, T>
    {
        FilteredEntityIter
        {
            inner: self,
            aspect: aspect,
            components: components,
        }
    }

    pub fn clone(&self) -> Self {
        let EntityIter::Map(ref values) = *self;
        EntityIter::Map(values.clone())
    }
}

impl<'a, T: ComponentManager> Iterator for EntityIter<'a, T>
{
    type Item = EntityData<'a, T>;
    fn next(&mut self) -> Option<EntityData<'a, T>>
    {
        match *self
        {
            EntityIter::Map(ref mut values) => values.next().map(|x| EntityData(x))
        }
    }
}

impl<'a, T: ComponentManager> Iterator for FilteredEntityIter<'a, T>
{
    type Item = EntityData<'a, T>;
    fn next(&mut self) -> Option<EntityData<'a, T>>
    {
        for x in self.inner.by_ref()
        {
            if self.aspect.check(&x, self.components)
            {
                return Some(x);
            }
            else
            {
                continue
            }
        }
        None
    }
}

enum Event
{
    BuildEntity(Entity),
    RemoveEntity(Entity),
}

/// Handles creation, activation, and validating of entities.
#[doc(hidden)]
pub struct EntityManager<T: ComponentManager>
{
    indices: IndexPool,
    entities: HashMap<Entity, IndexedEntity<T>>,
    event_queue: Vec<Event>,
    next_id: Id,
}

// TODO: Cleanup
#[cfg(feature="serialisation")]
unsafe impl<T: ComponentManager> CerealData for EntityManager<T> {
    fn write(&self, write: &mut ::std::io::Write) -> CerealResult<()> {
        if self.event_queue.len() != 0 {
            Err(CerealError::Msg("Please flush events before serialising the world".to_string()))
        } else {
            try!(self.indices.write(write));
            try!(self.entities.write(write));
            self.next_id.write(write)
        }
    }

    fn read(read: &mut ::std::io::Read) -> CerealResult<EntityManager<T>> {
        Ok(EntityManager {
            indices: try!(CerealData::read(read)),
            entities: try!(CerealData::read(read)),
            next_id: try!(CerealData::read(read)),
            event_queue: Vec::new(),
        })
    }
}

impl<T: ComponentManager> EntityManager<T>
{
    /// Returns a new `EntityManager`
    pub fn new() -> EntityManager<T>
    {
        EntityManager
        {
            indices: IndexPool::new(),
            entities: HashMap::new(),
            next_id: 0,
            event_queue: Vec::new(),
        }
    }

    pub fn flush_queue<M, S>(&mut self, c: &mut T, m: &mut M, s: &mut S)
    where M: ServiceManager, S: SystemManager<Components=T, Services=M>
    {
        let queue = ::std::mem::replace(&mut self.event_queue, Vec::new());
        for e in queue {
            match e {
                Event::BuildEntity(entity) => s.__activated(
                    EntityData(self.indexed(&entity)),
                    c,
                    m
                ),
                Event::RemoveEntity(entity) => {
                    {
                        let indexed = self.indexed(&entity);
                        s.__deactivated(EntityData(indexed), c, m);
                        c.__remove_all(indexed);
                    }
                    self.remove(&entity);
                }
            }
        }
    }

    pub fn create_entity<B>(&mut self, builder: B, c: &mut T) -> Entity where B: EntityBuilder<T>
    {
        let entity = self.create();
        builder.build(BuildData(self.indexed(&entity)), c);
        self.event_queue.push(Event::BuildEntity(entity));
        entity
    }

    pub fn remove_entity(&mut self, entity: Entity)
    {
        self.event_queue.push(Event::RemoveEntity(entity));
    }

    pub fn iter(&self) -> EntityIter<T>
    {
        EntityIter::Map(self.entities.values())
    }

    pub fn count(&self) -> usize
    {
        self.indices.count()
    }

    pub fn indexed(&self, entity: &Entity) -> &IndexedEntity<T>
    {
        &self.entities[entity]
    }

    /// Creates a new `Entity`, assigning it the first available index.
    pub fn create(&mut self) -> Entity
    {
        self.next_id += 1;
        let ret = Entity(self.next_id);
        self.entities.insert(ret, IndexedEntity(self.indices.get_index(), ret, PhantomData));
        ret
    }

    /// Returns true if an entity is valid (not removed from the manager).
    #[inline]
    pub fn is_valid(&self, entity: &Entity) -> bool
    {
        self.entities.contains_key(entity)
    }

    /// Deletes an entity from the manager.
    pub fn remove(&mut self, entity: &Entity)
    {
        self.entities.remove(entity).map(|e| self.indices.return_id(e.index()));
    }
}

struct IndexPool
{
    recycled: Vec<usize>,
    next_index: usize,
}

// TODO: Cleanup
#[cfg(feature="serialisation")]
unsafe impl CerealData for IndexPool {
    fn write(&self, write: &mut ::std::io::Write) -> CerealResult<()> {
        try!((self.recycled.len() as u64).write(write));
        for &idx in &self.recycled {
            try!((idx as u64).write(write));
        }
        (self.next_index as u64).write(write)
    }

    fn read(read: &mut ::std::io::Read) -> CerealResult<IndexPool> {
        let len = try!(u64::read(read)) as usize;
        let mut indices = Vec::with_capacity(len);
        for _ in 0..len {
            indices.push(try!(u64::read(read)) as usize);
        }
        Ok(IndexPool {
            recycled: indices,
            next_index: try!(u64::read(read)) as usize,
        })
    }
}


impl IndexPool
{
    pub fn new() -> IndexPool
    {
        IndexPool
        {
            recycled: Vec::new(),
            next_index: 0,
        }
    }

    pub fn count(&self) -> usize
    {
        self.next_index - self.recycled.len()
    }

    pub fn get_index(&mut self) -> usize
    {
        match self.recycled.pop()
        {
            Some(id) => id,
            None => {
                self.next_index += 1;
                self.next_index - 1
            }
        }
    }

    pub fn return_id(&mut self, id: usize)
    {
        self.recycled.push(id);
    }
}
